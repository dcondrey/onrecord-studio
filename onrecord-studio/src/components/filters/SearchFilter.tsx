import { formatMatchCount, SEARCH_PAGE_SIZE } from '../../utils/searchResults';
import type { SearchPageArgs } from '../../types';
import { useEffect, useMemo, useRef, useState } from 'react';
import AgencySelector from './AgencySelector';
import { useAppStore } from '../../store/useAppStore';
import { searchQueryId, searchRecordText, type SearchRecordInput } from '../../utils/searchResults';
import { formatNativeError, invokeCommand } from '../../ipc';
import { compileToLegacyQuery, interpretNaturalLanguage, parseQualifierQuery, serializeQualifierQuery } from '../../query';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';

export default function SearchFilter() {
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const evidenceRefreshRequests = useAppStore((state) => state.evidenceRefreshRequests);
  const searchTerm = useAppStore((state) => state.searchTerm);
  const selectedAgencyId = useAppStore((state) => state.selectedAgencyId);
  const selectedStatus = useAppStore((state) => state.selectedStatus);
  const selectedPolicyDeviation = useAppStore((state) => state.selectedPolicyDeviation);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setFilteredResults = useAppStore((state) => state.setFilteredResults);
  const error = useAppStore((state) => state.searchError);
  const setSearchError = useAppStore((state) => state.setSearchError);
  // The exact number of matches, not the number of rows loaded so far.
  const resultCount = useAppStore((state) => state.filteredResultsTotal);
  const retainedCount = useAppStore((state) => state.filteredResults.length);
  const agencyProfiles = useAppStore((state) => state.agencyProfiles);
  const portalCount = useAppStore((state) => state.portals.length);
  const [inputValue, setInputValue] = useState(searchTerm);
  const [interpretPlainLanguage, setInterpretPlainLanguage] = useState(false);
  const [isSearching, setIsSearching] = useState(false);
  const appliedRecord = useRef<SearchRecordInput | null>(null);
  const [watchStatus, setWatchStatus] = useState<{ ok: boolean; text: string } | null>(null);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const requestGate = useLatestRequestGate();
  // The parser accepts only fields the native command can execute. The compiler still returns
  // negated, duplicate, or ambiguous filters so they can be labeled instead of ignored.
  const parsedQuery = useMemo(
    () => interpretPlainLanguage ? interpretNaturalLanguage(inputValue) : parseQualifierQuery(inputValue),
    [inputValue, interpretPlainLanguage],
  );
  const compiledQuery = useMemo(
    () => compileToLegacyQuery(
      parsedQuery,
      agencyProfiles.map((profile) => ({
        agencyId: profile.agencyId,
        agencyName: profile.agencyName,
        portalKey: profile.portalSubdomain,
      })),
    ),
    [agencyProfiles, parsedQuery],
  );
  const deferredFilterKeys = useMemo(
    () => new Set(compiledQuery.deferredFilters.map(filterKey)),
    [compiledQuery.deferredFilters],
  );

  useEffect(() => {
    setInputValue(searchTerm);
  }, [searchTerm]);

  useEffect(() => {
    // LIMIT(query): Tauri invoke calls have no cancellation handle. Debouncing avoids work that has
    // not started; the latest-request gate prevents every stale completion from changing the UI.
    const request = requestGate.begin();
    const timer = window.setTimeout(async () => {
      setIsSearching(true);
      setSearchError(null);
      try {
        const args: SearchPageArgs = {
          searchTerm: compiledQuery.searchTerm,
          portalKey: compiledQuery.portalKey ?? activePortalKey,
          agencyId: compiledQuery.agencyId ?? selectedAgencyId,
          status: compiledQuery.status ?? selectedStatus,
          policyDeviation: compiledQuery.policyDeviation ?? selectedPolicyDeviation,
        };
        const page = await invokeCommand('query_audit_records_page', { ...args, cursor: null, pageSize: SEARCH_PAGE_SIZE });
        if (page.total === null) throw new Error('The search did not report how many records match.');
        if (requestGate.isCurrent(request)) {
          setFilteredResults(page.records, compiledQuery.searchTerm, { total: page.total, args, nextCursor: page.nextCursor });
          // Captured with the results it describes, so a record never pairs a newer query with an older count.
          appliedRecord.current = {
            ranAt: new Date().toISOString().slice(0, 16).replace('T', ' ') + ' UTC',
            query: inputValue,
            filters: [
              ...((compiledQuery.agencyId ?? selectedAgencyId) !== null ? [{ label: 'Department', value: agencyProfiles.find((profile) => profile.agencyId === (compiledQuery.agencyId ?? selectedAgencyId))?.agencyName ?? String(compiledQuery.agencyId ?? selectedAgencyId) }] : []),
              ...((compiledQuery.status ?? selectedStatus) ? [{ label: 'Status', value: String(compiledQuery.status ?? selectedStatus) }] : []),
              ...((compiledQuery.policyDeviation ?? selectedPolicyDeviation) ? [{ label: 'Policy deviation', value: formatPolicy(String(compiledQuery.policyDeviation ?? selectedPolicyDeviation)) }] : []),
              ...((compiledQuery.portalKey ?? activePortalKey) ? [{ label: 'Source', value: String(compiledQuery.portalKey ?? activePortalKey) }] : []),
            ],
            sourceCount: (compiledQuery.portalKey ?? activePortalKey) ? 1 : portalCount,
            resultCount: page.total,
          };
        }
      } catch (reason) {
        if (requestGate.isCurrent(request)) {
          // Keep the last successful result set. Clearing it turns a transport/query failure into
          // the false finding "no matches" and makes the visible filters look responsible.
          setSearchError(String(reason));
        }
      } finally {
        if (requestGate.isCurrent(request)) setIsSearching(false);
      }
    }, 220);
    return () => window.clearTimeout(timer);
  }, [activePortalKey, compiledQuery, evidenceRefreshRequests, inputValue, requestGate, selectedAgencyId, selectedPolicyDeviation, selectedStatus, setFilteredResults, setSearchError]);

  const updateTerm = (value: string) => {
    setInputValue(value);
    setSearchFilters(value, selectedAgencyId, selectedStatus, selectedPolicyDeviation);
  };

  const clear = () => {
    setInputValue('');
    setSearchFilters('', null, null, null);
  };

  const watchWords = compiledQuery.searchTerm.trim().split(/\s+/).filter(Boolean);
  const canWatch = watchWords.length >= 1 && watchWords.length <= 8;
  const watchTerm = async () => {
    const term = watchWords.join(' ');
    setWatchStatus(null);
    try {
      await invokeCommand('save_watch', {
        watch: { id: '', kind: 'QUERY', target: term, label: '', createdAt: '', enabled: true },
      });
      setWatchStatus({ ok: true, text: `Watching "${term}". It is checked after each ingest; changes appear in the watchlist.` });
    } catch (reason) {
      setWatchStatus({ ok: false, text: `Could not watch "${term}": ${formatNativeError(reason)}` });
    }
  };

  // A standing query stores the AST as written and the flat filters it compiled to. One that
  // compiles with deferred filters is refused here and again natively: saved, it would re-run as a
  // looser query than the one on screen.
  // The dropdown selections are part of the executed query, so they are part of the saved one.
  const standingFilters = {
    searchTerm: compiledQuery.searchTerm,
    portalKey: compiledQuery.portalKey ?? activePortalKey,
    agencyId: compiledQuery.agencyId ?? selectedAgencyId,
    status: compiledQuery.status ?? selectedStatus,
    policyDeviation: compiledQuery.policyDeviation ?? selectedPolicyDeviation,
  };
  const canStand = compiledQuery.deferredFilters.length === 0
    && (standingFilters.searchTerm.trim() !== '' || standingFilters.status !== null || standingFilters.policyDeviation !== null || standingFilters.agencyId !== null || standingFilters.portalKey !== null);
  const saveStanding = async () => {
    setWatchStatus(null);
    try {
      const saved = await invokeCommand('save_standing_query', {
        input: { label: inputValue.trim(), astJson: JSON.stringify(parsedQuery), filters: standingFilters },
      });
      setWatchStatus({ ok: true, text: `Standing query "${saved.label}" saved. It re-runs whenever the corpus changes; see Standing searches on the overview.` });
    } catch (reason) {
      setWatchStatus({ ok: false, text: `Could not save the standing query: ${formatNativeError(reason)}` });
    }
  };

  const copyRecord = async () => {
    const applied = appliedRecord.current;
    if (!applied) {
      setWatchStatus({ ok: false, text: 'No search has completed yet.' });
      return;
    }
    try {
      const [queryId, corpus] = await Promise.all([
        searchQueryId(applied.query, applied.filters),
        invokeCommand('get_corpus_state_digest', {}).catch(() => null),
      ]);
      await navigator.clipboard.writeText(searchRecordText({ ...applied, queryId, corpus }));
      setWatchStatus({ ok: true, text: 'Search record copied.' });
    } catch {
      setWatchStatus({ ok: false, text: 'The clipboard is unavailable.' });
    }
  };

  const hasFilters = Boolean(inputValue || selectedAgencyId || selectedStatus || selectedPolicyDeviation);

  return (
    <section aria-label="Evidence search" className="shrink-0 px-5 py-3 lg:px-6">
      <div className="app-panel flex flex-wrap items-center gap-2 p-2">
        <div className="relative min-w-[240px] flex-1">
          <label htmlFor="evidence-search" className="sr-only">Search requests and extracted documents</label>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-slate-400" aria-hidden="true">
            <circle cx="11" cy="11" r="7" /><path strokeLinecap="round" d="m20 20-4-4" />
          </svg>
          <input
            id="evidence-search"
            type="search"
            value={inputValue}
            onChange={(event) => updateTerm(event.target.value)}
            placeholder="Search request metadata and extracted text"
            className="focus-ring h-10 w-full rounded-xl border border-transparent bg-slate-950/65 pl-9 pr-10 text-[11px] text-slate-100 transition placeholder:text-slate-400 focus:border-indigo-400/50"
          />
          {isSearching && <span className="absolute right-3 top-1/2 h-3.5 w-3.5 -translate-y-1/2 animate-spin rounded-full border-2 border-slate-700 border-t-indigo-400" aria-label="Searching" />}
        </div>
        <AgencySelector />
        <label>
          <span className="sr-only">Request status</span>
          <select
            value={selectedStatus ?? ''}
            onChange={(event) => setSearchFilters(inputValue, selectedAgencyId, event.target.value || null, selectedPolicyDeviation)}
            className="focus-ring h-10 rounded-xl border border-slate-800 bg-slate-950/65 px-3 text-[10px] text-slate-300 transition focus:border-indigo-400/50"
          >
            <option value="">All statuses</option>
            <option value="open">Open</option>
            <option value="closed">Closed</option>
            <option value="referred">Referred</option>
          </select>
        </label>
        {hasFilters && (
          <button type="button" onClick={clear} className="focus-ring h-10 rounded-xl border border-slate-800 px-3 text-[10px] font-medium text-slate-400 transition hover:border-slate-700 hover:text-slate-200">
            Clear
          </button>
        )}
        <div className="ml-auto hidden shrink-0 border-l border-slate-800 px-2 text-right sm:block">
          <div className="font-mono text-[11px] font-semibold text-slate-300">{isSearching ? '—' : formatMatchCount(resultCount)}</div>
          <div className="text-[7px] uppercase tracking-wider text-slate-400">matches</div>
        </div>
      </div>
      <div className="mt-1.5 flex min-h-4 items-center gap-2 px-2 text-[8px]">
        <button
          type="button"
          aria-pressed={interpretPlainLanguage}
          onClick={() => setInterpretPlainLanguage((enabled) => !enabled)}
          className={interpretPlainLanguage ? 'rounded-full border border-indigo-400/30 bg-indigo-500/10 px-2 py-0.5 font-semibold text-indigo-200' : 'rounded-full border border-slate-800 px-2 py-0.5 font-semibold text-slate-400 hover:text-slate-300'}
        >
          Plain-language interpretation {interpretPlainLanguage ? 'on' : 'off'}
        </button>
        {parsedQuery.filters.map((filter, index) => {
          const deferred = deferredFilterKeys.has(filterKey(filter));
          return (
          <span key={`${filter.field}-${filter.value}-${index}`} title={deferred ? 'Recognized qualifier, but not applied to this search' : `Applied qualifier: ${filter.raw ?? serializeQualifierQuery({ version: 1, source: 'qualifier', text: '', filters: [filter], issues: [] })}`} className={deferred ? 'rounded-full border border-amber-500/25 bg-amber-500/[0.08] px-2 py-0.5 font-semibold text-amber-300' : 'rounded-full border border-cyan-500/20 bg-cyan-500/[0.06] px-2 py-0.5 font-semibold text-cyan-300'}>
            {filter.negated ? '-' : ''}{filter.field}:{filter.value}
          </span>
          );
        })}
        {compiledQuery.deferredFilters.length > 0 && (
          <span className="text-amber-300" role="status">
            {compiledQuery.deferredFilters.length} qualifier{compiledQuery.deferredFilters.length === 1 ? '' : 's'} not applied
          </span>
        )}
        {parsedQuery.issues.some((issue) => issue.severity === 'warning' || issue.severity === 'error') && (
          <span className="text-amber-300" title="Some query tokens need review">Query syntax needs review</span>
        )}
        {selectedPolicyDeviation && (
          <button type="button" onClick={() => setSearchFilters(inputValue, selectedAgencyId, selectedStatus, null)} className="rounded-full border border-amber-500/25 bg-amber-500/10 px-2 py-0.5 font-semibold text-amber-300">
            {formatPolicy(selectedPolicyDeviation)} ×
          </button>
        )}
        <button type="button" onClick={() => void copyRecord()} disabled={isSearching} className="rounded-full border border-slate-800 px-2 py-0.5 font-semibold text-slate-400 hover:text-slate-300">
          Copy search record
        </button>
        {canWatch && (
          <button type="button" onClick={() => void watchTerm()} title="Watches this text across all sources. The agency, status and policy filters are not part of a watch." className="rounded-full border border-slate-800 px-2 py-0.5 font-semibold text-slate-400 hover:text-slate-300">
            Watch this text for changes
          </button>
        )}
        {canStand && (
          <button type="button" onClick={() => void saveStanding()} disabled={isSearching} title="Saves this exact query, filters included. It re-runs whenever the corpus changes and records the corpus state each run saw." className="rounded-full border border-slate-800 px-2 py-0.5 font-semibold text-slate-400 hover:text-slate-300">
            Save as standing query
          </button>
        )}
        {watchStatus && (
          <span role={watchStatus.ok ? 'status' : 'alert'} className={watchStatus.ok ? 'text-emerald-300' : 'text-rose-300'}>
            {watchStatus.text}
            {watchStatus.ok && <button type="button" onClick={() => setWorkspaceView('overview')} className="ml-1 underline">Open watchlist</button>}
          </span>
        )}
        {error && (
          <span role="alert" className="truncate text-rose-300">
            Search unavailable; {retainedCount > 0 ? `showing ${formatMatchCount(retainedCount)} results from the last successful query` : 'no result set is available'}: {error}
          </span>
        )}
        {!error && parsedQuery.filters.length === 0 && <span className="text-slate-400">Search remains available while ingestion is active.</span>}
      </div>
    </section>
  );
}

function formatPolicy(value: string) {
  return value.toLowerCase().split('_').map((part) => part.charAt(0).toUpperCase() + part.slice(1)).join(' ');
}

function filterKey(filter: { field: string; operator: string; value: string; negated?: boolean }) {
  return `${filter.field}\u0000${filter.operator}\u0000${filter.value}\u0000${Boolean(filter.negated)}`;
}
