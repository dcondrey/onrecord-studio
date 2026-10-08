import { useState } from 'react';
import { useAppStore } from '../../store/useAppStore';
import { focusSourceComposer, openFileImport, SAN_DIEGO_RECORDS_URL, SAN_DIEGO_SOURCES } from '../../utils/sanDiego';
import Icon from '../ui/icons';
import StarterSets from './StarterSets';

type Source = (typeof SAN_DIEGO_SOURCES)[number];
const sourceCategory = (source: Source) => source.category ?? 'Government';
const sourceEvidenceType = (source: Source) => source.evidenceType ?? (source.category === 'News and reporting' ? 'Reporting' : 'Source collection');

export default function SanDiegoWorkspace() {
  const [sourceSearch, setSourceSearch] = useState('');
  const [category, setCategory] = useState('All sources');
  const [evidenceType, setEvidenceType] = useState('All evidence types');
  const categories = ['All sources', ...new Set(SAN_DIEGO_SOURCES.map(sourceCategory))];
  const evidenceTypes = ['All evidence types', ...new Set(SAN_DIEGO_SOURCES.map(sourceEvidenceType))];
  const visibleSources = SAN_DIEGO_SOURCES.filter((source) =>
    (category === 'All sources' || sourceCategory(source) === category)
    && (evidenceType === 'All evidence types' || sourceEvidenceType(source) === evidenceType)
    && `${source.label} ${source.publisher ?? ''} ${source.url} ${(source.aliases ?? []).join(' ')} ${source.description} ${source.analysisNotes ?? ''} ${source.period ?? ''}`.toLowerCase().includes(sourceSearch.trim().toLowerCase()),
  );
  const setPortalUrlDraft = useAppStore((state) => state.setPortalUrlDraft);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const portals = useAppStore((state) => state.portals);
  const initialization = useAppStore((state) => state.workspaceInitializationStatus);
  const stage = (url: string) => {
    setPortalUrlDraft(url);
    focusSourceComposer();
  };
  const search = (term: string) => {
    setActivePortal(null);
    setSearchFilters(term, null, null, null);
    setWorkspaceView('evidence');
  };

  return (
    <section aria-label="San Diego research desk" className="space-y-4">
      <div className="onboarding-panel rounded-2xl border p-5 sm:p-6">
        <div className="app-eyebrow text-accent">San Diego region · Research desk</div>
        <h3 className="mt-2 text-xl font-semibold tracking-tight text-fg">Start with the local record.</h3>
        <p className="mt-2 max-w-3xl text-sm leading-relaxed text-muted">Collect public records and regional data, inspect the evidence, and prepare a briefing. Choose a source below, then select Add in the toolbar. Import downloaded tables in Analysis to compare counts and trends.</p>
        <div className="mt-4 flex flex-wrap gap-2">
          {['homelessness', 'budget', 'use of force', 'public comment'].map((term) => (
            <button key={term} type="button" onClick={() => search(term)} className="secondary-action focus-ring rounded-lg border px-3 py-2 text-label leading-4 text-fg-secondary">Search “{term}”</button>
          ))}
          <button type="button" onClick={() => setWorkspaceView('briefing')} className="primary-action focus-ring rounded-lg px-3 py-2 text-label leading-4 font-semibold text-white">Prepare a briefing</button>
          <button type="button" onClick={() => setWorkspaceView('analysis')} className="secondary-action focus-ring rounded-lg border px-3 py-2 text-label leading-4 text-fg-secondary">Analyze a dataset</button>
        </div>
      </div>
      <StarterSets />
      <div className="flex flex-wrap items-center gap-3">
        <input aria-label="Find a regional source" value={sourceSearch} onChange={(event) => setSourceSearch(event.target.value)} placeholder="Find a source, agency, or topic" className="analysis-input min-w-64 flex-1" />
        <select aria-label="Source topic" value={category} onChange={(event) => setCategory(event.target.value)} className="analysis-input">
          {categories.map((value) => <option key={value}>{value}</option>)}
        </select>
        <select aria-label="Evidence type" value={evidenceType} onChange={(event) => setEvidenceType(event.target.value)} className="analysis-input">
          {evidenceTypes.map((value) => <option key={value}>{value}</option>)}
        </select>
        <span className="text-label leading-4 text-muted">{visibleSources.length} sources</span>
      </div>
      <div className="grid gap-3 md:grid-cols-2">
        {visibleSources.map((source) => {
          const saved = portals.find(({ registration }) => registration.canonicalUrl.replace(/\/$/, '') === source.url.replace(/\/$/, ''));
          return (
            <article key={source.id} className="app-panel flex flex-col p-5">
              <div className="flex items-center gap-2 text-sm font-semibold text-fg"><Icon name="filePlus" className="h-4 w-4 text-accent" />{source.label}</div>
              {source.publisher && <div className="mt-1 text-label leading-4 text-accent">{source.publisher} · {sourceCategory(source)}</div>}
              <p className="mt-2 text-label leading-relaxed text-muted">{source.description}</p>
              <p className="mt-2 text-label leading-relaxed text-muted">{source.coverage}</p>
              {source.analysisNotes && <p className="mt-2 text-label leading-relaxed text-fg-secondary">{source.analysisNotes}</p>}
              <div className="mt-4 flex flex-wrap items-center gap-2">
                {source.access === 'export'
                  ? <><button type="button" onClick={openFileImport} className="secondary-action focus-ring rounded-lg border px-3 py-2 text-label leading-4 font-semibold text-fg">Import downloaded files</button><button type="button" onClick={() => setWorkspaceView('analysis')} className="focus-ring rounded-lg px-3 py-2 text-label leading-4 text-accent">Analyze a table</button></>
                  : <button type="button" onClick={() => stage(source.url)} className="secondary-action focus-ring rounded-lg border px-3 py-2 text-label leading-4 font-semibold text-fg">{saved ? 'Update source' : 'Choose source'}</button>}
                <a href={source.url} target="_blank" rel="noreferrer" className="focus-ring rounded-lg px-3 py-2 text-label leading-4 text-accent">{new URL(source.url).pathname.toLowerCase().endsWith('.pdf') ? 'Open document' : 'Open website'}</a>
                {saved ? (
                  <button type="button" onClick={() => { setActivePortal(saved.registration.portalKey); setWorkspaceView('evidence'); }} className="focus-ring rounded-lg px-3 py-2 text-label leading-4 text-accent">Review saved records</button>
                ) : initialization === 'ready' ? <span className="text-label leading-4 text-muted">Not yet in your library</span> : null}
              </div>
            </article>
          );
        })}
      </div>
      {visibleSources.length === 0 && <p className="text-sm text-muted">No sources match. Try another topic or publisher.</p>}
      <div className="app-panel flex flex-col gap-3 p-5 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h3 className="text-sm font-semibold text-fg">City public-records requests</h3>
          <p className="mt-1 max-w-2xl text-label leading-relaxed text-muted">Choose a public-records source to discover public requests and attachments. Import downloaded response files when a portal requires a browser session. City, Housing Commission, County, and Sheriff’s records are held separately.</p>
        </div>
        <a href={SAN_DIEGO_RECORDS_URL} target="_blank" rel="noreferrer" className="secondary-action focus-ring shrink-0 rounded-lg border px-3 py-2 text-label leading-4 font-semibold text-fg">City public-records website</a>
      </div>
    </section>
  );
}
