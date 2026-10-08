import { useCallback, useEffect, useRef, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { Select } from '../ui/Field';
import type { FinancialCitation, FinancialMapping, FinancialReport, FinancialScenario, FinancialScope, FinancialWorkspace } from '../../types/financial';
import { financialAmount, financialReportMarkdown, rankFinancialLines, suggestFinancialMapping } from '../../utils/financial';
import BulletRows, { type BulletGroup, type BulletRow, type StageNode } from '../charts/figures/BulletRows';
import { formatCurrencyString } from '../charts/core/format';
import { absent, measured, type Measured } from '../charts/core/nulls';
import type { TwinCell } from '../charts/frame/TableTwin';
import type { FinancialChain, FinancialReconciliation } from '../../types/financial';

const labels: Record<string,string> = { account_label:'Account description', funding_duration:'Funding duration (one_time / recurring / mixed)',agency:'Responsible agency / jurisdiction',fy:'Fiscal year',fund:'Fund code',account:'Account code',center:'Funds/cost center',department:'Department',program:'Program',amount:'Amount / actual cost',basis:'Budget cycle / cost basis',contract_id:'Contract ID',vendor_id:'Vendor ID',transaction_id:'Transaction line ID',invoice_id:'Invoice ID',amendment_id:'Amendment ID',status:'Execution / posting status',period_start:'Period start (YYYY-MM-DD)',period_end:'Period end (YYYY-MM-DD)',population:'Population definition',metric:'Outcome / deliverable definition',unit:'Measurement unit',quantity:'Delivered quantity',denominator:'Eligible denominator',denominator_unit:'Denominator unit',canonical_account:'Canonical account code',end_fy:'Last effective fiscal year',restriction:'Restriction classification',eligible_program:'Expressly eligible program',transfer:'Transfer classification',currency:'Currency code',transaction_date:'Payment / transaction date (YYYY-MM-DD)',approval:'Approval status or approving action' };
const roleLabels: Record<string,string> = { budget_history:'Adopted funding history',actual_history:'Spending history', budget:'Appropriations',actual:'Actual expenditures',encumbrance:'Open encumbrances',contract:'Executed contracts',amendment:'Executed amendments',payment:'Posted payments',outcome:'Service outcomes',crosswalk:'Account crosswalk',restriction:'Fund restrictions' };
const emptyScope: FinancialScope = {fiscalYear:null,fund:null,department:null};
type Tab = 'reconcile'|'contracts'|'outcomes'|'scenarios'|'gaps';
/** `onCheckOrganisation` opens the organisation filings for a chain's vendor value, when the host offers it. */
export default function FinancialInvestigationsPanel({ onCheckOrganisation }: { onCheckOrganisation?: (vendor: string) => void } = {}) {
  const [workspace,setWorkspace]=useState<FinancialWorkspace|null>(null);
  const [scope,setScope]=useState<FinancialScope>(emptyScope);
  const scopeRef=useRef<FinancialScope>(emptyScope);
  const [report,setReport]=useState<FinancialReport|null>(null);
  const [scenario,setScenario]=useState<FinancialScenario|null>(null);
  const [copied,setCopied]=useState('');
  const [error,setError]=useState(''); const [pending,setPending]=useState(false); const [saving,setSaving]=useState(false); const [scenarioPending,setScenarioPending]=useState(false);
  const [tab,setTab]=useState<Tab>('reconcile'); const [mapping,setMapping]=useState<FinancialMapping|null>(null);
  const [selectedSources,setSelectedSources]=useState<string[]>([]);
  const connectionsRef=useRef<HTMLDetailsElement>(null);
  const [fundingKey,setFundingKey]=useState(''); const [outcomeKey,setOutcomeKey]=useState(''); const [targetProgram,setTargetProgram]=useState(''); const [proposedAmount,setProposedAmount]=useState('');
  const workspaceGate=useLatestRequestGate(); const reportGate=useLatestRequestGate(); const scenarioGate=useLatestRequestGate(); const copyGate=useLatestRequestGate();
  const run=useCallback(async(requestedScope:FinancialScope)=>{
    const token=reportGate.begin();scenarioGate.invalidate();copyGate.invalidate();setCopied('');setPending(true);setError('');setReport(null);setScenario(null);setSelectedSources([]);
    try { const next=await invokeCommand('run_financial_investigation',{scope:requestedScope}); if(!reportGate.isCurrent(token))return;setReport(next);setFundingKey(next.reconciliation[0]?.key??'');setOutcomeKey(next.outcomes[0]?.key??'');setTargetProgram(next.outcomes[0]?.program??''); }
    catch(reason){if(reportGate.isCurrent(token))setError(formatNativeError(reason));}finally{if(reportGate.isCurrent(token))setPending(false);}
  },[copyGate,reportGate,scenarioGate]);
  useEffect(()=>{
    if (!isTauri()) return;
    const token=workspaceGate.begin();
    void invokeCommand('get_financial_workspace',{}).then(next=>{if(!workspaceGate.isCurrent(token))return;setWorkspace(next);scopeRef.current=next.defaultScope;setScope(next.defaultScope);if(next.defaultScope.fiscalYear&&next.defaultScope.fund&&['budget','actual'].every(role=>next.mappings.some(m=>m.role===role)))void run(next.defaultScope);}).catch(reason=>{if(workspaceGate.isCurrent(token))setError(formatNativeError(reason));});
    return()=>{workspaceGate.invalidate();reportGate.invalidate();scenarioGate.invalidate();};
  },[workspaceGate,reportGate,scenarioGate,run]);
  const clearReport=()=>{copyGate.invalidate();setCopied('');reportGate.invalidate();scenarioGate.invalidate();setReport(null);setScenario(null);setPending(false);setScenarioPending(false);setSelectedSources([]);setFundingKey('');setOutcomeKey('');};
  const changeScope=(next:FinancialScope)=>{clearReport();scopeRef.current=next;setScope(next);};
  const save=async()=>{
    if(!mapping)return;clearReport();const token=workspaceGate.begin();setSaving(true);setError('');
    try { const next=await invokeCommand('save_financial_mapping',{mapping});if(!workspaceGate.isCurrent(token))return;setWorkspace(next);setMapping(null);if(scopeRef.current.fiscalYear&&scopeRef.current.fund)void run(scopeRef.current); }
    catch(reason){if(workspaceGate.isCurrent(token))setError(formatNativeError(reason));}finally{if(workspaceGate.isCurrent(token))setSaving(false);}
  };
  const scenarioChange=()=>{scenarioGate.invalidate();setScenario(null);setScenarioPending(false);};
  const calculate=async()=>{
    if(!report)return;const token=scenarioGate.begin();setScenarioPending(true);setScenario(null);setError('');
    try {const next=await invokeCommand('evaluate_financial_scenario',{input:{scope:report.scope,fundingKey,outcomeKey,targetProgram,proposedAmount}});if(!scenarioGate.isCurrent(token))return;setScenario(next);}
    catch(reason){if(scenarioGate.isCurrent(token))setError(formatNativeError(reason));}finally{if(scenarioGate.isCurrent(token))setScenarioPending(false);}
  };
  const evidence=(ids:string[])=> <button className="text-label leading-4 text-accent underline" onClick={()=>setSelectedSources(ids)}>Inspect {ids.length} cited rows</button>;
  const mappingDataset=workspace?.datasets.find(d=>d.snapshotId===mapping?.snapshotId);
  const openConnections=()=>{const details=connectionsRef.current;if(!details)return;details.open=true;details.scrollIntoView({block:'nearest'});details.querySelector('summary')?.focus();};
  const needsScope=!scope.fiscalYear||!scope.fund;
  return <section aria-label="Financial investigations" className="space-y-4">
    <div><h3 className="text-heading text-fg">Spending and outcomes</h3><p className="mt-1 max-w-prose text-caption text-muted">Compare budgeted, transferred and paid amounts with reported outcomes for the same period, using records already in your library.</p></div>
    {!isTauri()&&<p className="text-sm text-warning">Open the desktop application to query imported records.</p>}
    {error&&<p role="alert" className="text-sm text-danger">{error}</p>}
    {workspace&&<>
      {workspace.warnings.map(w=><p key={w} className="text-body text-warning">{w} <button type="button" className="focus-ring rounded text-accent underline-offset-2 hover:underline" onClick={openConnections}>Open record connections</button></p>)}
      <div className="flex flex-wrap items-end gap-3">
        <Select label="Fiscal year" aria-label="Financial fiscal year" className="min-w-[9rem]" value={scope.fiscalYear??''} onChange={e=>changeScope({...scope,fiscalYear:e.target.value?Number(e.target.value):null})}><option value="">Choose year</option>{workspace.fiscalYears.map(y=><option key={y}>{y}</option>)}</Select>
        <Select label="Fund" aria-label="Financial fund" className="min-w-[12rem]" value={scope.fund??''} onChange={e=>changeScope({...scope,fund:e.target.value||null})}><option value="">Choose fund</option>{workspace.funds.map(f=><option key={f}>{f}</option>)}</Select>
        <Select label="Department" aria-label="Financial department" className="min-w-[12rem]" value={scope.department??''} onChange={e=>changeScope({...scope,department:e.target.value||null})}><option value="">All departments</option>{workspace.departments.map(d=><option key={d}>{d}</option>)}</Select>
        <button className="analysis-button" disabled={pending||saving||needsScope} onClick={()=>void run(scope)}>{pending?'Recomputing cited comparisons…':'Investigate these records'}</button>
      </div>
      {needsScope&&<p className="-mt-2 text-caption text-muted">Choose a fiscal year and fund.</p>}
      <details ref={connectionsRef} className="rounded-lg border border-line p-3"><summary className="focus-ring w-fit cursor-pointer rounded text-control text-fg-secondary hover:text-fg">Record connections · {workspace.mappings.length} of {workspace.roles.length} types available</summary><p className="my-3 text-label leading-4 text-muted">Official City budget and actual exports connect automatically. For other ingested tables, review the exact source columns before applying a mapping. One snapshot per role prevents overlapping releases from being added together. Amounts use the source currency; blank fields remain unknown.</p>
        {!workspace.datasets.length&&<p className="text-sm text-warning">Ingest a budget, contract, payment or outcome table in the data workbench. This panel analyzes those imported records directly.</p>}
        <div className="grid gap-2 md:grid-cols-3">{workspace.roles.map(role=>{const current=workspace.mappings.find(m=>m.role===role);return <div key={role} className="rounded border border-line-strong p-2 text-sm"><strong>{roleLabels[role]??role}</strong><p className="text-label leading-4 text-muted">{workspace.datasets.find(d=>d.snapshotId===current?.snapshotId)?.name??'No record table connected'}</p><button disabled={saving||!workspace.datasets.length} className="mt-2 text-label leading-4 text-accent underline" onClick={()=>setMapping(current??suggestFinancialMapping(role,workspace.datasets[0],workspace.fields))}>Connect / review columns</button></div>;})}</div>
        {mapping&&<div className="mt-4 space-y-3 border-t border-line-strong pt-4"><h4 className="font-semibold">Map {roleLabels[mapping.role]}</h4><label className="text-sm">Imported snapshot<select aria-label="Financial source snapshot" disabled={saving} className="analysis-input ml-2" value={mapping.snapshotId} onChange={e=>{const d=workspace.datasets.find(d=>d.snapshotId===e.target.value);if(d)setMapping(suggestFinancialMapping(mapping.role,d,workspace.fields));}}>{workspace.datasets.map(d=><option value={d.snapshotId} key={d.snapshotId}>{d.name}{d.partial?' (partial)':''}</option>)}</select></label><div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-3">{workspace.fields.map(field=><label key={field} className="text-label leading-4">{labels[field]??field}{workspace.requirements[mapping.role]?.includes(field)?' *':''}<select aria-label={`Map ${field}`} disabled={saving} className="analysis-input block w-full" value={mapping.columns[field]??''} onChange={e=>{const columns={...mapping.columns};if(e.target.value)columns[field]=e.target.value;else delete columns[field];setMapping({...mapping,columns});}}><option value="">Not recorded</option>{mappingDataset?.columns.map(column=><option key={column}>{column}</option>)}</select></label>)}</div><label className="text-label leading-4">Agency when no source agency column exists<input aria-label="Declared source agency" disabled={saving} maxLength={150} className="analysis-input ml-2" placeholder="e.g. City of San Diego" value={mapping.agency??''} onChange={e=>setMapping({...mapping,agency:e.target.value||null})}/></label><label className="text-label leading-4">Currency when no source currency column exists<input aria-label="Declared source currency" disabled={saving} maxLength={3} className="analysis-input ml-2" placeholder="e.g. USD" value={mapping.currency??''} onChange={e=>setMapping({...mapping,currency:e.target.value.toUpperCase()||null})}/></label><p className="text-label leading-4 text-muted">Amounts represent amendment deltas, not replacement totals. Payment statuses must distinguish paid/posted/settled from void or pending. Outcome cost basis must explicitly be actual; restriction labels must state restricted or unrestricted. Funding duration accepts one_time, recurring or mixed; blank and unrecognized classifications stay unknown. Do not manufacture absent source fields.</p><div className="flex gap-3"><button className="analysis-button" disabled={saving} onClick={()=>void save()}>{saving?'Validating source columns…':'Apply source mapping'}</button><button disabled={saving} onClick={()=>setMapping(null)} className="text-sm text-fg-secondary">Cancel</button></div></div>}
      </details>
    </>}
    {report&&<>
      <div className="flex items-center gap-3"><button className="text-sm text-accent underline" onClick={()=>{const token=copyGate.begin();navigator.clipboard.writeText(financialReportMarkdown(report)).then(()=>{if(copyGate.isCurrent(token))setCopied('Report copied as Markdown.');},()=>{if(copyGate.isCurrent(token))setCopied('Copying was blocked by the system.');});}}>Copy report as Markdown</button>{copied&&<span role="status" className="text-label leading-4 text-muted">{copied}</span>}</div>
      <details open={report.findings.length>0} className="rounded-xl border border-line-strong p-3"><summary className="font-semibold">Investigative tests raised by these records</summary><p className="mt-2 text-label leading-4 text-muted">These are scoped hypotheses with competing explanations. An offset compares only the cited account pair; it does not establish a transfer or describe all departmental spending. Missing and incomplete evidence cannot establish a service cut.</p><div className="mt-3 grid gap-3 lg:grid-cols-2">{report.findings.slice(0,30).map(f=><article key={f.id} className="rounded-lg bg-surface/40 p-3"><h4 className="text-sm font-semibold">{f.title}</h4>{f.basis!=='lead'&&<p className="text-label leading-4 text-accent">{f.basis==='counted_comparison'?'Counted comparison of recorded rows with a written limit':'Labeled pattern, not a finding that a rule was broken'}{f.confidenceBasisPoints!=null&&` · confidence ${(f.confidenceBasisPoints/100).toFixed(0)}%`}</p>}<p className="my-2 text-sm text-fg-secondary">{f.observation}</p>{f.calculations.length>0&&<div className="my-3 space-y-2 rounded border border-line-strong p-2">{f.calculations.map(c=><div key={c.label}><p className="text-sm">{c.label}: <strong>{financialAmount(c.result,c.unit==='%'?'':c.unit)}{c.unit==='%'?'%':''}</strong></p><p className="break-words text-label leading-4 text-muted">{c.formula}</p></div>)}</div>}{evidence(f.citations)}<p className="mt-2 text-label leading-4 text-muted">Competing explanations: {f.alternatives.join(' ')}</p><p className="mt-2 text-sm text-accent">Next test: {f.nextTest}</p></article>)}</div>{!report.findings.length&&<p className="mt-2 text-sm text-muted">No supported audit pattern was established in this bounded scope. Missing-evidence requests remain available.</p>}</details>
      <div className="flex flex-wrap gap-2" role="group" aria-label="Financial investigation views">{([['reconcile','Budget reconciliation'],['contracts','Spending chains'],['outcomes','Service outcomes'],['scenarios','Policy scenarios'],['gaps','Missing evidence']] as [Tab,string][]).map(([value,label])=><button key={value} type="button" aria-pressed={tab===value} className={tab===value?'analysis-button':'focus-ring rounded px-3 py-2 text-sm text-fg-secondary'} onClick={()=>setTab(value)}>{label}</button>)}</div>
      {tab==='reconcile'&&<div className="space-y-3"><p className="text-label leading-4 text-muted">Stages remain separate. Adopted is the original plan; revised is current recorded authority. Unknown never means zero. Showing the 100 largest observed differences or amounts; narrow the department if the source exceeds the analysis limit.</p>{report.reconciliation.length>0&&<ReconcileChart lines={rankFinancialLines(report.reconciliation).slice(0,100)} onInspect={setSelectedSources}/>}<details className="rounded-lg border border-line p-3"><summary className="focus-ring w-fit cursor-pointer rounded text-control text-fg-secondary hover:text-fg">Exact amounts (copyable table)</summary><div className="mt-2 overflow-x-auto"><table className="w-full text-left text-label leading-4"><thead><tr>{['Account / center','Adopted','Revised','Actual','Encumbered','Actual − adopted','Revised − actual − encumbered','Evidence'].map(h=><th key={h} className="p-2">{h}</th>)}</tr></thead><tbody>{rankFinancialLines(report.reconciliation).slice(0,100).map(r=><tr key={r.key} className="border-t border-line-strong"><td className="p-2">{r.account} / {r.center||'unknown'}<p>{r.accountLabel}</p><div className="text-muted">{r.agency} · {r.department} · {r.currency||'currency unknown'} · funding: {r.fundingDuration}</div>{r.limitations.length>0&&<details><summary className="text-warning">Comparison limits</summary>{r.limitations.map(l=><p key={l}>{l}</p>)}</details>}</td>{[r.adopted,r.revised,r.actual,r.encumbered,r.actualMinusAdopted,r.uncommittedRevised].map((v,i)=><td key={i} className="whitespace-nowrap p-2 tabular-nums">{financialAmount(v)}</td>)}<td>{evidence(r.citations)}</td></tr>)}</tbody></table></div></details>{!report.reconciliation.length&&<p className="text-sm text-warning">No mapped financial rows match this fund and fiscal year. Review the source connections and missing-evidence list.</p>}</div>}
      {tab==='contracts'&&<div className="space-y-3"><p className="text-label leading-4 text-muted">Only stable contract identifiers link these stages. Contract chains include retrieved history through the selected year within this fund. Other funds and omitted earlier payments can change contract-wide balances. The stages are not added together; amendments are deltas. Missing linked outcomes are a coverage gap, not proof of failed delivery.</p>{report.chains.length>0&&<ContractChart chains={report.chains} paymentsMapped={workspace?.mappings.some(m=>m.role==='payment')??false} onInspect={setSelectedSources}/>}{report.chains.map((chain,i)=><article key={`${chain.contractId}-${i}`} className="rounded-xl border border-line-strong p-4"><h4 className="font-semibold">Contract {chain.contractId} · {chain.program||'program unresolved'}</h4><p className="text-label leading-4 text-muted">{chain.agency} · Vendor ID: {chain.vendor||'unknown'}{chain.vendor&&onCheckOrganisation&&<> · <button type="button" className="text-accent underline" onClick={()=>onCheckOrganisation(chain.vendor)}>Check this vendor's filings</button></>}</p>{[...chain.missingLinks,...chain.limitations].map(l=><p key={l} className="mt-2 text-label leading-4 text-warning">{l}</p>)}</article>)}{!report.chains.length&&<p className="text-sm text-warning">No contract identifiers are available to connect authority, payments and outcomes. Department totals cannot substitute for a contract register.</p>}</div>}
      {tab==='outcomes'&&<div className="grid gap-3 lg:grid-cols-2">{report.outcomes.map(o=><article key={o.key} className="rounded-xl border border-line-strong p-4"><h4 className="font-semibold">{o.program} · {o.metric||'metric unknown'}</h4><p className="text-label leading-4 text-muted">{o.agency} · {o.periodStart||'unknown start'}–{o.periodEnd||'unknown end'} · {o.population||'population unknown'}</p><p className="mt-3 text-sm">Reported {o.unit||'units'}: {o.quantity??'Unknown'}; actual cost: {financialAmount(o.cost,o.currency)}</p><p className="text-sm">Cost per {o.unit||'unit'}: {financialAmount(o.costPerUnit,o.currency)}</p><p className="text-sm">Eligible denominator: {o.denominator??'Unknown'} {o.denominatorUnit}; measured rate: {o.successRatePercent==null?'Unknown':`${o.successRatePercent}%`}</p><p className="my-2 text-label leading-4 text-muted">Comparable only with matching metric, unit, population, period, currency and cost basis. Case mix and quality require separate review.</p>{evidence(o.citations)}{o.limitations.map(l=><p key={l} className="mt-2 text-label leading-4 text-warning">{l}</p>)}</article>)}{!report.outcomes.length&&<p className="text-sm text-warning">No outcome table is connected for this period. Link measured deliverables, actual costs, units, population and dates to calculate meaningful comparisons.</p>}</div>}
      {tab==='scenarios'&&<div className="space-y-3"><p className="text-sm text-fg-secondary">Estimate service capacity for the selected fiscal year only, from a cited uncommitted revised balance and comparable observed costs. Funding restrictions and missing commitments can block the calculation. This does not authorize a transfer or forecast outcomes.</p><div className="grid gap-3 md:grid-cols-2"><label className="text-sm">Funding line<select aria-label="Scenario funding line" className="analysis-input block w-full" value={fundingKey} onChange={e=>{scenarioChange();setFundingKey(e.target.value);}}><option value="">Choose funding line</option>{report.reconciliation.map(r=><option key={r.key} value={r.key}>{r.account} / {r.center} · {r.department} · {r.fundingDuration} funding · balance {financialAmount(r.uncommittedRevised,r.currency)}</option>)}</select></label><label className="text-sm">Observed service cost<select aria-label="Scenario outcome baseline" className="analysis-input block w-full" value={outcomeKey} onChange={e=>{scenarioChange();setOutcomeKey(e.target.value);}}><option value="">Choose observed service</option>{report.outcomes.map(o=><option key={o.key} value={o.key}>{o.program} · {o.metric} · {financialAmount(o.costPerUnit,o.currency)}/{o.unit}</option>)}</select></label><label className="text-sm">Target program<input aria-label="Scenario target program" className="analysis-input block w-full" value={targetProgram} onChange={e=>{scenarioChange();setTargetProgram(e.target.value);}}/></label><label className="text-sm">Proposed amount<input aria-label="Scenario amount" inputMode="decimal" className="analysis-input block w-full" value={proposedAmount} onChange={e=>{scenarioChange();setProposedAmount(e.target.value);}}/></label></div><button className="analysis-button" disabled={scenarioPending||!fundingKey||!outcomeKey||!proposedAmount||!targetProgram} onClick={()=>void calculate()}>{scenarioPending?'Rechecking sources and restrictions…':'Calculate sourced scenario'}</button>{scenario&&<div className="rounded-xl border border-line-strong p-4" aria-label="Scenario result"><p className="text-sm text-accent">{scenario.periodScope} · funding: {scenario.fundingDuration}</p><h4 className="font-semibold">{scenario.status==='blocked'?'Scenario blocked by unresolved evidence':`Conditional estimate: ${scenario.estimatedWholeUnits} ${scenario.unit}`}</h4>{scenario.status!=='blocked'&&<p className="text-sm">Observed comparable range: {scenario.lowerUnits}–{scenario.upperUnits} whole units.</p>}<p className="my-2 break-words text-label leading-4 text-fg-secondary">{scenario.formula}</p>{scenario.blockers.map(b=><p key={b} className="text-sm text-warning">{b}</p>)}{scenario.assumptions.map(a=><p key={a} className="mt-2 text-label leading-4 text-muted">{a}</p>)}<details className="mt-3"><summary className="text-sm text-accent">Scenario source rows</summary>{scenario.citations.map(c=><Source key={c.id} source={c} queries={scenario.queries}/>)}</details></div>}</div>}
      {tab==='gaps'&&<div className="grid gap-3 md:grid-cols-2">{report.gaps.map((g,i)=><article key={`${g.title}-${i}`} className="rounded-xl border border-line-strong p-4"><h4 className="font-semibold">{g.title}</h4><p className="mt-2 text-sm">{g.recordsNeeded}</p><p className="mt-2 text-label leading-4 text-muted">{g.why}</p></article>)}</div>}

      {selectedSources.length>0&&<section aria-label="Financial source evidence" className="rounded-xl border border-accent p-4"><h4 className="font-semibold">Exact rows and reproducible calculations</h4>{report.evidence.filter(e=>selectedSources.includes(e.id)).map(source=><Source key={source.id} source={source} queries={report.queries}/>)}</section>}
      <details className="text-label leading-4 text-muted"><summary>Coverage and interpretation limits</summary>{report.limitations.map((l,i)=><p key={i} className="mt-2">{l}</p>)}</details>
    </>}
  </section>;
}
function Source({source,queries}:{source:FinancialCitation;queries:Record<string,string>}) {
  return <details className="mt-3 rounded border border-line-strong p-3 text-label leading-4"><summary className="text-accent">{source.role} · snapshot {source.snapshotId}</summary>{source.sourceUrl&&<a className="mt-2 block break-all text-accent underline" href={source.sourceUrl} target="_blank" rel="noreferrer">Original published record</a>}<p className="my-2 break-all text-muted">Original SHA-256: {source.sourceSha256}</p><div className="overflow-x-auto"><table className="text-left"><tbody>{source.columns.map((column,i)=><tr key={column}><th className="pr-4 align-top font-normal text-muted">{column}</th><td className="break-all">{source.row[i]===null?'Unknown':String(source.row[i])}</td></tr>)}</tbody></table></div><details className="mt-3"><summary>Executed read-only SQL</summary><pre className="mt-2 overflow-auto whitespace-pre-wrap">{queries[source.queryId]??'Unavailable'}</pre></details></details>;
}

const STAGES: [string, string][] = [['budget', 'Budget'], ['contract', 'Contract'], ['amendment', 'Amendment'], ['payment', 'Payment'], ['outcome', 'Outcome']];
const negative = (v: string | null) => v !== null && v.trim().startsWith('-') && /[1-9]/.test(v);
const positive = (v: string | null) => v !== null && !v.trim().startsWith('-') && /[1-9]/.test(v);
/** A printed amount from its decimal string; an unrecorded currency prints no symbol. */
function money(v: string, currency: string, options: { compact?: boolean; signed?: boolean } = {}): string {
  const text = formatCurrencyString(v, { ...options, currency: currency || 'USD' });
  return currency ? text : text.replace('$', '');
}
const plotted = (v: string | null, absence: Measured): Measured => (v === null ? absence : measured(Number(v)));
const exact = (v: string | null, currency: string, reason: string): TwinCell => (v === null ? absent('not-recorded', reason) : financialAmount(v, currency));
const currencyLabel = (currency: string) => (currency ? `Amounts in ${currency}` : 'Currency not recorded: amounts on their own axis');

/** Groups in first-seen order, one per key; a group never mixes currencies. */
function grouped<T>(items: readonly T[], keyOf: (item: T) => { key: string; label: string; currency: string }, rowOf: (item: T) => BulletRow): BulletGroup[] {
  const groups = new Map<string, BulletGroup>();
  for (const item of items) {
    const { key, label, currency } = keyOf(item);
    const group = groups.get(key) ?? { key, label, currency, rows: [] };
    group.rows.push(rowOf(item));
    groups.set(key, group);
  }
  return [...groups.values()];
}

function ContractChart({ chains, paymentsMapped, onInspect }: { chains: FinancialChain[]; paymentsMapped: boolean; onInspect: (ids: string[]) => void }) {
  const compared = chains.filter((chain) => chain.remainingAuthority !== null);
  const over = compared.filter((chain) => negative(chain.remainingAuthority)).sort((a, b) => Number(a.remainingAuthority) - Number(b.remainingAuthority));
  const overBy = (chain: FinancialChain, signed = true) => money((chain.remainingAuthority as string).trim().replace(/^-/, ''), chain.currency, { signed });
  const excluded = chains.length - compared.length;
  const headline = !paymentsMapped
    ? `No payments dataset mapped, so none of ${chains.length} ${chains.length === 1 ? 'contract is' : 'contracts are'} compared with posted payments.`
    : `${over.length ? `${over.length} of ${compared.length}` : `None of ${compared.length}`} contracts show posted payments above retrieved authority${over.length ? `; largest: ${over[0].contractId} by ${overBy(over[0], false)}` : ''}${excluded ? `; ${excluded} ${excluded === 1 ? 'lacks' : 'lack'} retrieved authority or payments` : ''}.`;
  const groups = grouped(chains, (chain) => ({ key: chain.currency, label: currencyLabel(chain.currency), currency: chain.currency }), (chain): BulletRow => {
    const label = `Contract ${chain.contractId} · ${chain.program || 'program unresolved'}`;
    const fig = (v: string | null) => (v === null ? 'not retrieved' : financialAmount(v, chain.currency));
    const paid = chain.payments === null ? (paymentsMapped ? 'payments not retrieved' : 'no payments dataset mapped') : `${money(chain.payments, chain.currency, { compact: true })} paid`;
    const authority = chain.totalAuthority === null ? 'authority not retrieved' : `${money(chain.totalAuthority, chain.currency, { compact: true })} authority`;
    const stages: StageNode[] = STAGES.map(([role, stageLabel]) => {
      const ids = chain.stages[role];
      return { key: role, label: stageLabel, state: ids === undefined ? 'not-retrieved' : ids.length ? 'linked' : 'not-linked', count: ids?.length ?? 0, onOpen: ids?.length ? () => onInspect(ids) : undefined };
    });
    return {
      key: `${chain.contractId}:${chain.currency}`,
      label,
      track: plotted(chain.totalAuthority, absent('not-recorded', 'no award or amendment documents retrieved')),
      base: chain.baseAuthority === null ? undefined : measured(Number(chain.baseAuthority)),
      bar: paymentsMapped ? plotted(chain.payments, absent('not-recorded', 'no posted payments retrieved')) : absent('not-covered', 'No payments dataset mapped'),
      ticks: chain.baseAuthority === null ? [] : [{ key: 'base', at: measured(Number(chain.baseAuthority)), label: `Base authority ${financialAmount(chain.baseAuthority, chain.currency)}` }],
      barLabel: `${paid} / ${authority}`,
      overrunLabel: negative(chain.remainingAuthority) ? `${overBy(chain)} over retrieved authority` : undefined,
      name: `${label}: base authority ${fig(chain.baseAuthority)}, amendment deltas ${fig(chain.amendments)}, current authority ${fig(chain.totalAuthority)}, posted payments ${paymentsMapped ? fig(chain.payments) : 'no payments dataset mapped'}.`,
      tooltip: {
        title: label,
        rows: [
          { key: 'base', value: fig(chain.baseAuthority), label: 'base authority' },
          { key: 'amend', value: fig(chain.amendments), label: 'amendment deltas' },
          { key: 'total', value: fig(chain.totalAuthority), label: 'current authority' },
          { key: 'paid', value: paymentsMapped ? fig(chain.payments) : 'not mapped', label: 'posted payments' },
        ],
        note: 'Amendments are deltas; stages are not summed.',
      },
      stages,
    };
  });
  return (
    <BulletRows
      title="Contract authority and posted payments"
      headline={headline}
      sources={{ tables: ['financial_mappings', 'analytical_rows (contract, amendment and payment snapshots)'], note: 'amendments are deltas; stages are not summed' }}
      summary={`${chains.length} contracts in ${groups.length} currency ${groups.length === 1 ? 'group' : 'groups'}. ${over.length} with posted payments above retrieved authority. ${excluded} without both figures.${paymentsMapped ? '' : ' No payments dataset is mapped.'}`}
      status="ready"
      table={{
        caption: 'Contract authority and payments',
        columns: [{ key: 'contract', label: 'Contract' }, { key: 'base', label: 'Base authority', align: 'end' }, { key: 'amend', label: 'Amendment deltas', align: 'end' }, { key: 'total', label: 'Current authority', align: 'end' }, { key: 'paid', label: 'Posted payments', align: 'end' }, { key: 'remaining', label: 'Authority − payments', align: 'end' }],
        rows: chains.map((chain) => ({
          contract: `${chain.contractId} · ${chain.program || 'program unresolved'}`,
          base: exact(chain.baseAuthority, chain.currency, 'no award document retrieved'),
          amend: exact(chain.amendments, chain.currency, 'no amendments retrieved'),
          total: exact(chain.totalAuthority, chain.currency, 'no award or amendment documents retrieved'),
          paid: paymentsMapped ? exact(chain.payments, chain.currency, 'no posted payments retrieved') : absent('not-covered', 'No payments dataset mapped'),
          remaining: exact(chain.remainingAuthority, chain.currency, 'needs both authority and payments'),
        })),
      }}
      legend={[
        { key: 'track', label: 'Retrieved authority', swatch: 'rect', tone: 'track' },
        { key: 'base', label: 'Base authority', swatch: 'line', tone: 'ink' },
        { key: 'bar', label: 'Posted payments', swatch: 'rect', tone: 'ink' },
        { key: 'linked', label: 'Stage linked (cited rows)', swatch: 'dot', tone: 'ink' },
        { key: 'unlinked', label: 'Stage not linked', swatch: 'hollow', tone: 'ink' },
      ]}
      groups={groups}
      storageKey="financial-contract-bullets"
    />
  );
}

const RECONCILE_PLOTTED = 20;

function ReconcileChart({ lines, onInspect }: { lines: FinancialReconciliation[]; onInspect: (ids: string[]) => void }) {
  const [all, setAll] = useState(false);
  const shown = all ? lines : lines.slice(0, RECONCILE_PLOTTED);
  const both = lines.filter((line) => line.actualMinusAdopted !== null);
  const above = both.filter((line) => positive(line.actualMinusAdopted)).length;
  const noActual = lines.filter((line) => line.actual === null).length;
  const headline = `${above} of ${both.length} lines spent above adopted; ${noActual} ${noActual === 1 ? 'line lacks' : 'lines lack'} an actual figure.`;
  const groups = grouped(shown, (line) => ({ key: `${line.department}\u0000${line.currency}`, label: `${line.department || 'Department not recorded'} · ${line.currency || 'currency not recorded'}`, currency: line.currency }), (line): BulletRow => {
    const label = `${line.account} / ${line.center || 'unknown'} · ${line.accountLabel}`;
    const fig = (v: string | null) => (v === null ? 'not recorded' : financialAmount(v, line.currency));
    const delta = line.actualMinusAdopted === null ? null : `${money(line.actualMinusAdopted, line.currency, { compact: true, signed: true })} vs adopted`;
    const actual = line.actual === null ? 'no actual' : `${money(line.actual, line.currency, { compact: true })} actual`;
    return {
      key: line.key,
      label,
      track: plotted(line.revised, absent('not-recorded', 'no revised authority recorded')),
      bar: plotted(line.actual, absent('not-recorded', 'no actual recorded')),
      ticks: line.adopted === null ? [] : [{ key: 'adopted', at: measured(Number(line.adopted)), label: `Adopted ${financialAmount(line.adopted, line.currency)}` }],
      // An absent actual is labelled by its slot; a second label would sit on top of it.
      barLabel: line.actual === null ? undefined : delta ? `${actual} · ${delta}` : actual,
      overrunLabel: `${actual} · above revised`,
      name: `${label}: adopted ${fig(line.adopted)}, revised ${fig(line.revised)}, actual ${fig(line.actual)}, encumbered ${fig(line.encumbered)}.${line.citations.length ? ' Press Enter to inspect its cited rows.' : ''}`,
      tooltip: {
        title: label,
        rows: [
          { key: 'adopted', value: fig(line.adopted), label: 'adopted' },
          { key: 'revised', value: fig(line.revised), label: 'revised' },
          { key: 'actual', value: fig(line.actual), label: 'actual' },
          { key: 'enc', value: fig(line.encumbered), label: 'encumbered' },
          { key: 'delta', value: line.actualMinusAdopted === null ? 'not computed' : money(line.actualMinusAdopted, line.currency, { signed: true }), label: 'actual − adopted' },
        ],
        note: line.citations.length ? `${line.citations.length} cited rows; click to inspect.` : undefined,
      },
      onOpen: line.citations.length ? () => onInspect(line.citations) : undefined,
    };
  });
  return (
    <div className="grid gap-2">
      <BulletRows
        title="Actual against revised authority, by department"
        headline={headline}
        sources={{ tables: ['financial_mappings', 'analysis_fin_budget', 'analysis_fin_actual', 'analysis_fin_encumbrance'], note: 'over or under adopted is not a finding by itself' }}
        summary={`${lines.length} lines, ${shown.length} plotted in ${groups.length} department groups. ${above} above adopted; ${noActual} without an actual figure.`}
        status="ready"
        table={{
          caption: 'Budget reconciliation lines',
          columns: [{ key: 'line', label: 'Account / center' }, { key: 'adopted', label: 'Adopted', align: 'end' }, { key: 'revised', label: 'Revised', align: 'end' }, { key: 'actual', label: 'Actual', align: 'end' }, { key: 'enc', label: 'Encumbered', align: 'end' }, { key: 'delta', label: 'Actual − adopted', align: 'end' }],
          rows: shown.map((line) => ({
            line: `${line.account} / ${line.center || 'unknown'} · ${line.department}`,
            adopted: exact(line.adopted, line.currency, 'no adopted amount recorded'),
            revised: exact(line.revised, line.currency, 'no revised amount recorded'),
            actual: exact(line.actual, line.currency, 'no actual recorded'),
            enc: exact(line.encumbered, line.currency, 'no encumbrance recorded'),
            delta: exact(line.actualMinusAdopted, line.currency, 'needs both adopted and actual'),
          })),
        }}
        legend={[
          { key: 'track', label: 'Revised authority', swatch: 'rect', tone: 'track' },
          { key: 'bar', label: 'Actual', swatch: 'rect', tone: 'ink' },
          { key: 'tick', label: 'Adopted', swatch: 'line', tone: 'ink' },
        ]}
        groups={groups}
        storageKey="financial-reconcile-bullets"
      />
      {lines.length > RECONCILE_PLOTTED && (
        <button type="button" className="focus-ring w-fit rounded text-caption text-accent underline-offset-2 hover:underline" aria-pressed={all} onClick={() => setAll((current) => !current)}>
          {all ? `Show the ${RECONCILE_PLOTTED} largest lines` : `Show all ${lines.length} lines`}
        </button>
      )}
    </div>
  );
}
