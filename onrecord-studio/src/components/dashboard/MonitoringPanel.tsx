import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../ipc';
import type { CoverageExpectation, MonitoringEvidence, MonitoringStatus, ServiceObservation } from '../../types/monitoring';
import { citationLocation, observationPoints, optionalMeasurement } from '../../utils/monitoring';

const inputClass = 'w-full rounded-lg border border-slate-700 bg-slate-950 p-2 text-sm text-slate-100';
const panelClass = 'rounded-xl border border-slate-700 bg-slate-950/40 p-4';

function Citation({ source }: { source: MonitoringEvidence }) {
  return <details className="mt-2 text-label leading-4 text-slate-400"><summary className="cursor-pointer">Inspect source and identity</summary>
    <a className="mt-2 block text-cyan-300 underline" href={source.sourceUrl} target="_blank" rel="noreferrer">{source.title}</a>
    <p className="mt-1">{citationLocation(source)} · SHA-256 {source.sourceSha256.slice(0, 16)}…</p>
    <blockquote className="mt-2 max-h-48 overflow-auto whitespace-pre-wrap border-l-2 border-slate-700 pl-3 text-slate-300">{source.quote}</blockquote>
  </details>;
}

export default function MonitoringPanel({ caseId = null, topic = 'homelessness' }: { caseId?: string | null; topic?: string }) {
  const [result, setStatus] = useState<MonitoringStatus | null>(null);
  const status = result?.topic === topic && result.caseId === caseId ? result : null;
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [tab, setTab] = useState<'coverage' | 'changes' | 'services'>('coverage');
  const [newCoverage, setNewCoverage] = useState(false);
  const [newObservation, setNewObservation] = useState(false);
  const [selectedObservation, setSelectedObservation] = useState<string | null>(null);
  const [dateFrom, setDateFrom] = useState(''); const [dateTo, setDateTo] = useState('');
  const identity = `${topic}:${caseId ?? ''}`;
  const identityRef = useRef(identity); identityRef.current = identity;
  const generation = useRef(0);

  async function load(refresh = false) {
    const requestedIdentity = identityRef.current; const requestedGeneration = ++generation.current;
    setBusy(true); setError(null);
    try {
      const next = await invokeCommand(refresh ? 'refresh_monitoring_now' : 'get_monitoring_status', { topic, caseId });
      if (identityRef.current !== requestedIdentity || generation.current !== requestedGeneration) return;
      setStatus(next);
    } catch (failure) {
      if (identityRef.current !== requestedIdentity || generation.current !== requestedGeneration) return;
      setError(String(failure));
    } finally {
      if (identityRef.current === requestedIdentity && generation.current === requestedGeneration) setBusy(false);
    }
  }

  useEffect(() => {
    setStatus(null); setError(null); setNewCoverage(false); setNewObservation(false); setSelectedObservation(null);
    const requestedIdentity = identity; const requestedGeneration = ++generation.current;
    setBusy(true);
    void invokeCommand('get_monitoring_status', { topic, caseId }).then(next => {
      if (identityRef.current !== requestedIdentity || generation.current !== requestedGeneration) return;
      setStatus(next);
    }).catch(failure => {
      if (identityRef.current !== requestedIdentity || generation.current !== requestedGeneration) return;
      setError(String(failure));
    }).finally(() => {
      if (identityRef.current === requestedIdentity && generation.current === requestedGeneration) setBusy(false);
    });
    return () => { generation.current += 1; };
  }, [identity, topic, caseId]);

  async function save(kind: 'coverage' | 'observation', value: CoverageExpectation | ServiceObservation) {
    const requestedIdentity = identityRef.current; const requestedGeneration = ++generation.current;
    setBusy(true); setError(null);
    try {
      if (kind === 'coverage') await invokeCommand('save_monitoring_coverage', { expectation: value as CoverageExpectation });
      else await invokeCommand('save_monitoring_observation', { observation: value as ServiceObservation });
      if (identityRef.current !== requestedIdentity || generation.current !== requestedGeneration) return;
      setNewCoverage(false); setNewObservation(false);
      await load();
    } catch (failure) {
      if (identityRef.current !== requestedIdentity || generation.current !== requestedGeneration) return;
      setError(String(failure)); setBusy(false);
    }
  }

  const observations = (status?.observations ?? []).filter(o => (!dateFrom || o.observedOn >= dateFrom) && (!dateTo || o.observedOn <= dateTo));
  const points = observationPoints(observations);
  const selected = observations.find(o => o.id === selectedObservation);
  return <section aria-label="Evidence monitoring" className="app-panel p-5 sm:p-6">
    <div className="flex flex-wrap items-start justify-between gap-3"><div><div className="app-eyebrow text-cyan-300">Records that move the investigation</div>
      <h3 className="mt-2 text-xl font-semibold text-slate-100">Missing records, changing commitments and service access</h3>
      <p className="mt-2 max-w-3xl text-sm text-slate-400">Turn unresolved questions into acquisition work. Compare cited versions and reviewed location observations before drawing conclusions.</p></div>
      <button className="analysis-button" disabled={busy} onClick={() => void load(true)}>{busy ? 'Checking records…' : 'Check ingested records'}</button></div>
    <div className="mt-4 flex flex-wrap gap-2" role="tablist" aria-label="Monitoring views">{(['coverage', 'changes', 'services'] as const).map(value => <button role="tab" aria-selected={tab === value} className={`analysis-button ${tab === value ? 'border-cyan-400 text-cyan-200' : ''}`} key={value} onClick={() => setTab(value)}>{value === 'coverage' ? 'Acquisition priorities' : value === 'changes' ? 'Publication changes' : 'Service access and operations'}</button>)}</div>
    {error && <p role="alert" className="mt-3 text-sm text-rose-300">{error}</p>}
    {!status && busy && <p role="status" className="mt-4 text-sm text-slate-400">Loading this investigation’s evidence…</p>}
    {status && tab === 'coverage' && <div className="mt-4 space-y-3">
      <button className="analysis-button" onClick={() => setNewCoverage(!newCoverage)}>Define a consequential missing record</button>
      {newCoverage && <CoverageForm caseId={caseId} topic={topic} busy={busy} onSave={value => void save('coverage', value)} />}
      {!status.coverage.length && <p className="text-sm text-slate-400">Define the records needed to resolve a specific question. Requests from saved research dossiers also appear here automatically.</p>}
      {status.coverage.map(item => <article key={item.expectation.id} className={panelClass}>
        <div className="text-label leading-4 text-amber-200">{item.status.replace(/_/g, ' ')} · {item.expectation.agency} · {item.expectation.period}</div>
        <h4 className="mt-2 font-semibold text-slate-100">{item.expectation.recordType}</h4>
        <p className="mt-2 text-sm text-slate-300">Unlocks: {item.expectation.consequence}</p><p className="mt-2 text-label leading-4 text-slate-400">{item.explanation}</p>
        <details className="mt-3"><summary className="cursor-pointer text-sm text-cyan-300">Acquisition or records-request draft</summary><textarea aria-label={`Acquisition draft for ${item.expectation.recordType}`} className={`${inputClass} mt-2 min-h-32`} readOnly value={item.expectation.acquisitionAction} /></details>
        {item.candidates.map(candidate => <div key={`${candidate.portalKey}:${candidate.documentId}`} className="mt-3 border-t border-slate-800 pt-3"><Citation source={candidate} />
          <button className="analysis-button mt-2" disabled={busy} onClick={() => void save('coverage', { ...item.expectation, caseId, state: 'RECEIVED', evidence: [candidate] })}>Record receipt after checking scope and period</button></div>)}
        {item.expectation.state !== 'EXPECTED' && <button className="analysis-button mt-3" disabled={busy} onClick={() => void save('coverage', { ...item.expectation, state: 'EXPECTED' })}>Reopen acquisition gap</button>}
      </article>)}
    </div>}
    {status && tab === 'changes' && <div className="mt-4 space-y-3"><p className="text-label leading-4 text-slate-400">Observed retrieval versions across the monitored corpus. Changes are review leads; a failed fetch is never treated as deletion.</p>
      {!status.changes.length && <p className="text-sm text-slate-400">No version differences have been observed since monitoring began. The next accepted extraction of a changed source will be compared with its compact baseline.</p>}
      {status.changes.map(change => <article className={panelClass} key={change.id}><div className="text-label leading-4 text-amber-200">{change.category.replace(/_/g, ' ')} · Observed {new Date(change.observedAt).toLocaleString()}</div>
        <a className="mt-2 block font-semibold text-cyan-200 underline" href={change.sourceUrl} target="_blank" rel="noreferrer">{change.title}</a>
        <div className="mt-3 grid gap-3 md:grid-cols-2"><div><h5 className="text-label leading-4 font-semibold text-slate-400">Previously retrieved</h5><blockquote className="mt-1 whitespace-pre-wrap text-sm text-slate-300">{change.beforeQuote ?? 'No corresponding statement in the bounded baseline.'}</blockquote><p className="mt-2 break-all text-label leading-4 text-slate-400">SHA-256 {change.beforeSha256}</p></div><div><h5 className="text-label leading-4 font-semibold text-slate-400">Subsequently retrieved</h5><blockquote className="mt-1 whitespace-pre-wrap text-sm text-slate-300">{change.afterQuote ?? 'Not present in the subsequently retrieved bounded material.'}</blockquote><p className="mt-2 break-all text-label leading-4 text-slate-400">SHA-256 {change.afterSha256}</p></div></div><p className="mt-3 text-label leading-4 text-slate-400">{change.interpretation}</p></article>)}
    </div>}
    {status && tab === 'services' && <div className="mt-4 space-y-4"><div className="flex flex-wrap gap-3"><label className="text-label leading-4 text-slate-400">From<input type="date" className={inputClass} value={dateFrom} onChange={e => setDateFrom(e.target.value)} /></label><label className="text-label leading-4 text-slate-400">Through<input type="date" className={inputClass} value={dateTo} onChange={e => setDateTo(e.target.value)} /></label><button className="analysis-button self-end" onClick={() => setNewObservation(!newObservation)}>Add a sourced observation</button></div>
      {newObservation && <ObservationForm topic={topic} caseId={caseId} sources={status.sourceCandidates} busy={busy} onSave={value => void save('observation', value)} />}
      <p className="text-label leading-4 text-slate-400">Explicit coordinates only. Gray points await source review. Blank hours and coordinates mean unknown. Reports or complaints are not automatically treated as completed operations.</p>
      {points.length > 0 ? <div className={panelClass}><svg viewBox="0 0 600 300" role="img" aria-label="Geographic positions recorded in the sources" className="max-h-80 w-full rounded-lg bg-slate-900"><path d="M30 10V270H590" fill="none" stroke="#475569" /><text x="300" y="295" fill="#94a3b8" fontSize="10">Longitude → · relative coordinate plot, not a route map</text><text x="5" y="15" fill="#94a3b8" fontSize="10">N</text>{points.map(({ observation: o, x, y }) => <g key={o.id} role="button" tabIndex={0} aria-label={`${o.place}, ${o.observedOn}`} onClick={() => setSelectedObservation(o.id)} onKeyDown={event => { if (event.key === 'Enter' || event.key === ' ') setSelectedObservation(o.id); }}><circle cx={x} cy={y} r={selectedObservation === o.id ? 8 : 5} fill={!o.reviewed ? '#64748b' : o.kind === 'FACILITY' ? '#22d3ee' : '#fbbf24'} /><title>{o.place} · {o.observedOn} · {o.latitude}, {o.longitude}</title></g>)}</svg>{selected && <div className="mt-3 text-sm text-slate-200">{selected.place} · {selected.observedOn}<Citation source={selected.evidence} /></div>}</div> : <p className="text-sm text-slate-400">No sourced coordinates in this date range. Import a public facility inventory or operation table, or add an observation with an exact source citation.</p>}
      {status.comparisons.map((comparison, index) => <article className={panelClass} key={`${comparison.kind}:${index}`}><h4 className="font-semibold text-slate-100">{comparison.title}</h4><p className="mt-2 text-sm text-cyan-200">{comparison.calculation}</p><p className="mt-2 text-sm text-slate-300">{comparison.implication}</p><p className="mt-2 text-label leading-4 text-slate-400">{comparison.limitations.join(' ')}</p>{comparison.observationIds.map(id => { const observation = status.observations.find(o => o.id === id); return observation ? <Citation key={id} source={observation.evidence} /> : null; })}</article>)}
      <div className="max-h-[36rem] space-y-3 overflow-auto">{observations.map(observation => <article className={panelClass} key={observation.id}><div className="text-label leading-4 text-slate-400">{observation.kind} · {observation.observedOn} · {observation.reviewed ? 'Reviewed for comparison' : 'Needs review'}</div><h4 className="mt-1 font-semibold text-slate-100">{observation.place}</h4><p className="mt-1 text-label leading-4 text-slate-400">Hours/week: {observation.hoursPerWeek ?? 'Unknown'} · Coordinates: {observation.latitude === null ? 'Unknown' : `${observation.latitude}, ${observation.longitude}`} · {observation.status}</p><Citation source={observation.evidence} /><button disabled={busy || observation.status === 'SOURCE_CHANGED'} className="analysis-button mt-3" onClick={() => void save('observation', { ...observation, caseId, topic, reviewed: !observation.reviewed })}>{observation.reviewed ? 'Exclude from comparisons' : 'Confirm source and include in this case'}</button></article>)}</div>
    </div>}
    {status && <details className="mt-4 text-label leading-4 text-slate-400"><summary className="cursor-pointer">Coverage and interpretation limits</summary>{status.limitations.map(limit => <p className="mt-2" key={limit}>{limit}</p>)}</details>}
  </section>;
}

function CoverageForm({ caseId, topic, busy, onSave }: { caseId: string | null; topic: string; busy: boolean; onSave: (value: CoverageExpectation) => void }) {
  return <form className={`${panelClass} grid gap-3 md:grid-cols-2`} onSubmit={event => { event.preventDefault(); const data = new FormData(event.currentTarget); const get = (key: string) => String(data.get(key) ?? '').trim(); onSave({ id: '', caseId, topic, agency: get('agency'), recordType: get('recordType'), period: get('period'), queryTerms: get('terms').split(',').map(s => s.trim()).filter(Boolean), consequence: get('consequence'), acquisitionAction: get('action'), state: 'EXPECTED', evidence: [] }); }}>
    {[['agency', 'Custodian or agency'], ['recordType', 'Specific record type'], ['period', 'Required fiscal or observation period'], ['terms', 'Search terms, separated by commas']].map(([name, label]) => <label className="text-label leading-4 text-slate-400" key={name}>{label}<input required name={name} className={inputClass} /></label>)}
    <label className="text-label leading-4 text-slate-400 md:col-span-2">What comparison or question will this unlock?<textarea required name="consequence" className={inputClass} /></label><label className="text-label leading-4 text-slate-400 md:col-span-2">Acquisition plan or draft request for existing records<textarea required name="action" className={`${inputClass} min-h-24`} /></label><button disabled={busy} className="analysis-button" type="submit">Save acquisition priority</button>
  </form>;
}

function ObservationForm({ topic, caseId, sources, busy, onSave }: { topic: string; caseId: string | null; sources: MonitoringEvidence[]; busy: boolean; onSave: (value: ServiceObservation) => void }) {
  const [sourceIndex, setSourceIndex] = useState(0); const [error, setError] = useState<string | null>(null);
  const source = sources[sourceIndex];
  return <form className={`${panelClass} grid gap-3 md:grid-cols-2`} onSubmit={event => { event.preventDefault(); if (!source) return; const data = new FormData(event.currentTarget); const get = (key: string) => String(data.get(key) ?? '').trim(); try { onSave({ id: '', caseId, topic, kind: get('kind'), place: get('place'), observedOn: get('date'), latitude: optionalMeasurement(get('latitude')), longitude: optionalMeasurement(get('longitude')), hoursPerWeek: optionalMeasurement(get('hours')), status: get('status') || 'NOT_RECORDED', evidence: source, reviewed: true }); } catch (failure) { setError(String(failure)); } }}>
    <label className="text-label leading-4 text-slate-400 md:col-span-2">Supporting public record<select className={inputClass} value={sourceIndex} onChange={event => setSourceIndex(Number(event.target.value))}>{sources.map((candidate, index) => <option value={index} key={`${candidate.portalKey}:${candidate.documentId}`}>{candidate.title}</option>)}</select></label>
    {source && <div className="md:col-span-2"><Citation source={source} /></div>}
    <label className="text-label leading-4 text-slate-400">Observation type<select name="kind" className={inputClass}><option value="FACILITY">Public facility access</option><option value="OPERATION">Documented operation</option><option value="OBSERVATION">Other observation / report</option></select></label>
    {[['place', 'Recorded public place', 'text'], ['date', 'Observation date in the source', 'date'], ['latitude', 'Latitude explicitly recorded (optional)', 'text'], ['longitude', 'Longitude explicitly recorded (optional)', 'text'], ['hours', 'Hours per week (blank if unknown)', 'text'], ['status', 'Recorded operating status', 'text']].map(([name, label, type]) => <label className="text-label leading-4 text-slate-400" key={name}>{label}<input name={name} type={type} required={name === 'place' || name === 'date'} className={inputClass} /></label>)}
    {error && <p role="alert" className="text-label leading-4 text-rose-300">{error}</p>}<button type="submit" className="analysis-button" disabled={busy || !source}>Save reviewed observation</button>
  </form>;
}
