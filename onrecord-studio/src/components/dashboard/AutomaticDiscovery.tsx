import { useState } from 'react';
import { useAppStore } from '../../store/useAppStore';
import { metadataObservations, profileObservations, rankObservations } from '../../utils/automaticDiscovery';
import { focusSourceComposer } from '../../utils/sanDiego';
import { useAutomaticDiscovery } from './AutomaticDiscoveryProvider';
import Icon from '../ui/icons';
import DataMaintenance from './DataMaintenance';

export default function AutomaticDiscovery() {
  const { controller, state } = useAutomaticDiscovery();
  const [showAll, setShowAll] = useState(false);
  const store = useAppStore;
  if (!state || !controller) return null;
  const datasets = state.schema?.datasets ?? [];
  const observations = datasets.flatMap((dataset) => [
    ...metadataObservations(dataset),
    ...(state.profiles[dataset.snapshotId] ? profileObservations(state.schema!, dataset, state.profiles[dataset.snapshotId]) : []),
  ]);
  const ranked = rankObservations(observations, showAll ? 30 : 6);
  const profileErrors = Object.values(state.profiles).flatMap((profile) => profile.results.filter((entry) => entry.error));
  const stage = (url: string) => { store.getState().setPortalUrlDraft(url); focusSourceComposer(); };
  const openAnalysis = () => store.getState().setWorkspaceView('analysis');
  return <section className="app-panel space-y-4 p-4 sm:p-5" aria-label="Automatic discoveries">
    <div className="flex flex-wrap items-start justify-between gap-3">
      <div><div className="app-eyebrow flex items-center gap-2 text-cyan-300"><Icon name="sparkles" className="h-4 w-4" /> Across your library</div>
        <h2 className="mt-1 text-lg font-semibold text-slate-100">What stands out</h2>
        <p className="mt-1 text-xs text-slate-400">Automatic observations and follow-up questions from your tables and indexed documents.</p>
      </div>
      <button type="button" className="analysis-button" onClick={controller.refresh}>Refresh discoveries</button>
    </div>
    <p role="status" className="text-xs text-slate-400">{!state.checkedAt ? 'Reading your library…' : state.paused ? 'Automatic checks are waiting for your current analysis.' : state.activeSnapshot ? `Checking ${datasets.find((dataset) => dataset.snapshotId === state.activeSnapshot)?.displayName ?? 'an imported table'}…` : 'Monitoring for new data.'}
      {datasets.length > 0 && ` ${Object.values(state.profiles).filter((profile) => profile.results.some((entry) => entry.result)).length} of ${datasets.length} snapshots have automatic results.`}</p>
    {Object.entries(state.errors).map(([key, error]) => <p key={key} role="alert" className="text-xs text-amber-200">{key} check unavailable: {error}. Previously returned observations may be stale.</p>)}
    {profileErrors.length > 0 && <details className="text-xs text-amber-200"><summary>{profileErrors.length} checks need another attempt</summary>{profileErrors.map((entry, i) => <p key={i}>{entry.query.title}: {entry.error}</p>)}<p>Other checks continue. Failed checks retry after a delay.</p></details>}
    <div className="grid gap-3 md:grid-cols-2 xl:grid-cols-3">
      {ranked.map((item) => {
        const dataset = datasets.find((candidate) => candidate.snapshotId === item.snapshotId)!;
        return <article key={item.id} className="rounded-xl border border-slate-700/70 bg-slate-900/20 p-3">
          <div className="text-[10px] uppercase tracking-wide text-cyan-300">{item.kind === 'pattern' ? 'Observed pattern' : item.kind === 'quality' ? 'Data quality' : 'Coverage'}</div>
          <h3 className="mt-1 text-sm font-semibold text-slate-100">{item.title}</h3>
          <p className="mt-2 text-xs leading-relaxed text-slate-300">{item.detail}</p>
          <p className="mt-2 text-xs leading-relaxed text-cyan-200">Investigate: {item.question}</p>
          <p className="mt-2 break-words text-[10px] text-slate-400">{dataset.displayName} · {dataset.sourceUri ?? 'Local export'}{dataset.partialDataset ? ' · Partial / unverified' : ''}</p>
          <details className="mt-1 text-[10px] text-slate-400"><summary>Evidence and method</summary><p className="break-all">Snapshot {dataset.snapshotId}<br />SHA-256 {dataset.sourceSha256}</p>{item.query && <p>{item.query.method}</p>}</details>
          {item.query && <button type="button" className="analysis-button mt-2" onClick={() => { controller.inspect(item.query!.sql); openAnalysis(); }}>Explore this finding</button>}
        </article>;
      })}
      {state.patterns.slice(0, showAll ? 8 : 3).map((pattern) => <article key={pattern.patternId} className="rounded-xl border border-slate-700/70 p-3">
        <div className="text-[10px] uppercase tracking-wide text-cyan-300">Across indexed documents</div><h3 className="mt-1 text-sm font-semibold text-slate-100">{pattern.title}</h3>
        <p className="mt-2 text-xs leading-relaxed text-slate-300">{pattern.explanation}</p>
        <p className="mt-2 text-xs text-slate-400">{pattern.evidence.length} returned evidence references. Shared language or entities do not establish a causal connection.</p>
        <button type="button" className="analysis-button mt-2" onClick={() => { const actions = store.getState(); actions.setActivePortal(null); actions.setPatternEvidenceSelection({ patternId: pattern.patternId, portalKey: pattern.evidence[0]?.portalKey ?? null, documentId: pattern.evidence[0]?.documentId ?? null, characterStart: pattern.evidence[0]?.characterStart ?? null }); actions.setWorkspaceView('patterns'); }}>Inspect document evidence</button>
      </article>)}
    </div>
    {observations.length > 6 && <button type="button" className="analysis-button" onClick={() => setShowAll((value) => !value)}>{showAll ? 'Show fewer observations' : 'Show more observations'}</button>}
    {state.candidates.length > 0 && <p className="text-xs text-slate-300">{state.candidates.length} acquired tables are available for analysis. <button type="button" className="text-cyan-300 underline" onClick={openAnalysis}>Review acquired tables</button></p>}
    {!datasets.length && !state.patterns.length && <div className="grid gap-3 text-xs sm:grid-cols-2">
      <div className="rounded-xl border border-slate-800 p-3"><h3 className="font-semibold text-slate-200">Where does funding reach the community?</h3><p className="mt-1 text-slate-400">Start with operating-budget records. Separate adopted allocations from proposals, then compare with actuals for matching years.</p><button type="button" className="analysis-button mt-2" onClick={() => stage('https://data.sandiego.gov/datasets/operating-budget')}>Choose budget data</button></div>
      <div className="rounded-xl border border-slate-800 p-3"><h3 className="font-semibold text-slate-200">Do housing outcomes match the reported investment?</h3><p className="mt-1 text-slate-400">Bring in performance reports and preserve each measure’s population and period. The app will surface connections as records arrive.</p><button type="button" className="analysis-button mt-2" onClick={() => stage('https://www.rtfhsd.org/reports-data/')}>Choose housing reports</button></div>
      <p className="text-slate-400 sm:col-span-2">These are starting questions, not findings about data that has not been imported.</p>
    </div>}
    {datasets.length > 0 && !ranked.length && !state.activeSnapshot && <p className="text-xs text-slate-400">No highlighted pattern was produced by the completed checks. Automatic checks cover up to 12 prioritized columns per table; deeper discovery remains available in Analysis.</p>}
    <DataMaintenance />
  </section>;
}
