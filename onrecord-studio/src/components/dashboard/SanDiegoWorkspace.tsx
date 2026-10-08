import { useState } from 'react';
import { useAppStore } from '../../store/useAppStore';
import { focusSourceComposer, openFileImport, SAN_DIEGO_RECORDS_URL, SAN_DIEGO_SOURCES } from '../../utils/sanDiego';
import Button, { buttonClass } from '../ui/Button';
import { Input, Select } from '../ui/Field';
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
      <p className="max-w-prose text-body text-muted">Choose a source below, then select Add to collect it. Import downloaded tables in Analysis to compare counts and trends.</p>
      <p className="flex flex-wrap items-center gap-x-3 gap-y-1 text-body text-muted">
        Try a search:
        {['homelessness', 'budget', 'use of force', 'public comment'].map((term) => (
          <Button key={term} variant="link" onClick={() => search(term)}>{term}</Button>
        ))}
      </p>
      {portals.length > 0 && <StarterSets />}
      <div className="grid items-end gap-3 sm:grid-cols-[minmax(0,1fr)_auto_auto_auto]">
        <Input aria-label="Find a regional source" value={sourceSearch} onChange={(event) => setSourceSearch(event.target.value)} placeholder="Find a source, agency or topic" />
        <Select aria-label="Source topic" value={category} onChange={(event) => setCategory(event.target.value)}>
          {categories.map((value) => <option key={value}>{value}</option>)}
        </Select>
        <Select aria-label="Evidence type" value={evidenceType} onChange={(event) => setEvidenceType(event.target.value)}>
          {evidenceTypes.map((value) => <option key={value}>{value}</option>)}
        </Select>
        <span className="pb-2 text-caption tabular-nums text-muted">{visibleSources.length} source{visibleSources.length === 1 ? '' : 's'}</span>
      </div>
      <div className="grid gap-4 md:grid-cols-2">
        {visibleSources.map((source) => {
          const saved = portals.find(({ registration }) => registration.canonicalUrl.replace(/\/$/, '') === source.url.replace(/\/$/, ''));
          return (
            <article key={source.id} className="flex flex-col rounded-lg border border-line p-4">
              <h4 className="text-body font-medium text-fg">{source.label}</h4>
              {source.publisher && <p className="mt-0.5 text-caption text-muted">{source.publisher} · {sourceCategory(source)}</p>}
              <p className="mt-2 text-body text-muted">{source.description}</p>
              <p className="mt-2 text-caption text-muted">{source.coverage}</p>
              {source.analysisNotes && <p className="mt-2 text-caption text-fg-secondary">{source.analysisNotes}</p>}
              <div className="mt-auto flex flex-wrap items-center gap-2 pt-4">
                {source.access === 'export'
                  ? <><Button size="sm" onClick={openFileImport}>Import downloaded files</Button><Button size="sm" variant="ghost" onClick={() => setWorkspaceView('analysis')}>Analyze a table</Button></>
                  : <Button size="sm" onClick={() => stage(source.url)}>{saved ? 'Update source' : 'Choose source'}</Button>}
                <a href={source.url} target="_blank" rel="noreferrer" className={buttonClass({ variant: 'ghost', size: 'sm', className: 'focus-ring' })}>{new URL(source.url).pathname.toLowerCase().endsWith('.pdf') ? 'Open document' : 'Open website'}<span className="sr-only"> (opens in browser)</span></a>
                {saved ? (
                  <Button size="sm" variant="ghost" onClick={() => { setActivePortal(saved.registration.portalKey); setWorkspaceView('evidence'); }}>Review saved records</Button>
                ) : initialization === 'ready' ? <span className="text-caption text-muted">Not yet in your library</span> : null}
              </div>
            </article>
          );
        })}
      </div>
      {visibleSources.length === 0 && <p className="text-body text-muted">No sources match. Try another topic or publisher.</p>}
      <div className="flex flex-col gap-3 rounded-lg border border-line p-4 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <h4 className="text-body font-medium text-fg">City public-records requests</h4>
          <p className="mt-1 max-w-prose text-caption text-muted">Choose a public-records source to discover public requests and attachments. Import downloaded response files when a portal requires a browser session. City, Housing Commission, County, and Sheriff’s records are held separately.</p>
        </div>
        <a href={SAN_DIEGO_RECORDS_URL} target="_blank" rel="noreferrer" className={buttonClass({ className: 'focus-ring' })}>City public-records website<span className="sr-only"> (opens in browser)</span></a>
      </div>
    </section>
  );
}
