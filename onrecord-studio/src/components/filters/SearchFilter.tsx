import { useEffect, useMemo, useState } from 'react';
import AgencySelector from './AgencySelector';
import { useAppStore } from '../../store/useAppStore';
import { invokeCommand } from '../../ipc';
import { compileToLegacyQuery, parseQualifierQuery, serializeQualifierQuery } from '../../query';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';

export default function SearchFilter() {
  const activePortalKey = useAppStore((state) => state.activePortalKey);
  const searchTerm = useAppStore((state) => state.searchTerm);
  const selectedAgencyId = useAppStore((state) => state.selectedAgencyId);
  const selectedStatus = useAppStore((state) => state.selectedStatus);
  const selectedPolicyDeviation = useAppStore((state) => state.selectedPolicyDeviation);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setFilteredResults = useAppStore((state) => state.setFilteredResults);
  const error = useAppStore((state) => state.searchError);
  const setSearchError = useAppStore((state) => state.setSearchError);
  const resultCount = useAppStore((state) => state.filteredResults.length);
  const agencyProfiles = useAppStore((state) => state.agencyProfiles);
  const [inputValue, setInputValue] = useState(searchTerm);
  const [isSearching, setIsSearching] = useState(false);
  const requestGate = useLatestRequestGate();
  // NOTE(query): `parsedQuery` is currently presentation-only. The legacy IPC
  // call below intentionally remains unchanged until deferred filters can be
  // enforced by Rust; see docs/query-language.md before wiring execution.
  const parsedQuery = useMemo(() => parseQualifierQuery(inputValue), [inputValue]);
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
    // PERF(query): Replace debounce-only protection with AbortController once
    // the native command bridge accepts cancellation; requestSequence guards
    // against stale responses for now.
    const request = requestGate.begin();
    const timer = window.setTimeout(async () => {
      setIsSearching(true);
      setSearchError(null);
      try {
        const results = await invokeCommand('query_audit_records', {
          searchTerm: compiledQuery.searchTerm,
          portalKey: compiledQuery.portalKey ?? activePortalKey,
          agencyId: compiledQuery.agencyId ?? selectedAgencyId,
          status: compiledQuery.status ?? selectedStatus,
          policyDeviation: compiledQuery.policyDeviation ?? selectedPolicyDeviation,
          limit: 500,
        });
        if (requestGate.isCurrent(request)) setFilteredResults(results);
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
  }, [activePortalKey, compiledQuery, inputValue, requestGate, selectedAgencyId, selectedPolicyDeviation, selectedStatus, setFilteredResults, setSearchError]);

  const updateTerm = (value: string) => {
    // FEATURE(query): Add visual filter rows and autocomplete suggestions while
    // preserving this text field as the canonical qualifier editor.
    setInputValue(value);
    setSearchFilters(value, selectedAgencyId, selectedStatus, selectedPolicyDeviation);
  };

  const clear = () => {
    setInputValue('');
    setSearchFilters('', null, null, null);
  };

  const hasFilters = Boolean(inputValue || selectedAgencyId || selectedStatus || selectedPolicyDeviation);

  return (
    <section aria-label="Evidence search" className="shrink-0 px-5 py-3 lg:px-6">
      <div className="app-panel flex flex-wrap items-center gap-2 p-2">
        <div className="relative min-w-[240px] flex-1">
          <label htmlFor="evidence-search" className="sr-only">Search requests and extracted documents</label>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="1.8" className="pointer-events-none absolute left-3 top-1/2 h-4 w-4 -translate-y-1/2 text-slate-500" aria-hidden="true">
            <circle cx="11" cy="11" r="7" /><path strokeLinecap="round" d="m20 20-4-4" />
          </svg>
          <input
            id="evidence-search"
            type="search"
            value={inputValue}
            onChange={(event) => updateTerm(event.target.value)}
            placeholder="Search request metadata and extracted text"
            className="focus-ring h-10 w-full rounded-xl border border-transparent bg-slate-950/65 pl-9 pr-10 text-[11px] text-slate-100 transition placeholder:text-slate-600 focus:border-indigo-400/50"
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
          <div className="font-mono text-[11px] font-semibold text-slate-300">{isSearching ? '—' : resultCount.toLocaleString()}</div>
          <div className="text-[7px] uppercase tracking-wider text-slate-700">matches</div>
        </div>
      </div>
      <div className="mt-1.5 flex min-h-4 items-center gap-2 px-2 text-[8px]">
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
        {error && (
          <span role="alert" className="truncate text-rose-300">
            Search unavailable; {resultCount > 0 ? `showing ${resultCount.toLocaleString()} results from the last successful query` : 'no result set is available'}: {error}
          </span>
        )}
        {!error && parsedQuery.filters.length === 0 && <span className="text-slate-600">Search remains available while WAL ingestion is active.</span>}
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
