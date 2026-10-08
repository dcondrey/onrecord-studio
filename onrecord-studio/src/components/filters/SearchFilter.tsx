import { formatMatchCount, SEARCH_PAGE_SIZE } from '../../utils/searchResults';
import type { SearchPageArgs } from '../../types';
import { useEffect, useMemo, useRef, useState } from 'react';
import AgencySelector from './AgencySelector';
import { useAppStore } from '../../store/useAppStore';
import { searchQueryId, searchRecordText, type SearchRecordInput } from '../../utils/searchResults';
import { invokeCommand } from '../../ipc';
import { compileToLegacyQuery, interpretNaturalLanguage, parseQualifierQuery, serializeQualifierQuery } from '../../query';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { displayError } from '../../utils/errorMessage';
import { SearchField, Select } from '../ui/Field';
import { buttonClass } from '../ui/Button';
import Switch from '../ui/Switch';

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
  const setSearchStatus = useAppStore((state) => state.setSearchStatus);
  // Until a query has answered, a count is unknown, not zero.
  const countKnown = useAppStore((state) => state.searchStatus === 'ready');
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
      setSearchStatus('pending');
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
          setSearchError(displayError(reason).message);
        }
      } finally {
        if (requestGate.isCurrent(request)) setIsSearching(false);
      }
    }, 220);
    return () => window.clearTimeout(timer);
  }, [activePortalKey, compiledQuery, evidenceRefreshRequests, inputValue, requestGate, selectedAgencyId, selectedPolicyDeviation, selectedStatus, setFilteredResults, setSearchError, setSearchStatus]);

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
      setWatchStatus({ ok: false, text: `Could not watch "${term}": ${displayError(reason).message}` });
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
      setWatchStatus({ ok: false, text: `Could not save the standing query: ${displayError(reason).message}` });
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
      <div className="app-panel flex items-center gap-2 p-2">
        <SearchField
          id="evidence-search"
          aria-label="Search requests and extracted documents"
          value={inputValue}
          onChange={(event) => updateTerm(event.target.value)}
          placeholder="Search requests and document text"
          wrapperClassName="min-w-0 flex-1"
          trailing={isSearching ? <span className="h-3 w-3 shrink-0 animate-spin rounded-full border-2 border-line-strong border-t-accent" role="status" aria-label="Searching" /> : null}
        />
        <AgencySelector />
        <Select
          aria-label="Request status"
          value={selectedStatus ?? ''}
          onChange={(event) => setSearchFilters(inputValue, selectedAgencyId, event.target.value || null, selectedPolicyDeviation)}
          className="focus-ring w-32 xl:w-36"
        >
          <option value="">All statuses</option>
          <option value="open">Open</option>
          <option value="closed">Closed</option>
          <option value="referred">Referred</option>
        </Select>
        {/* Always mounted so the field does not shrink under the caret on the first keystroke. */}
        <button type="button" onClick={clear} aria-hidden={!hasFilters || undefined} tabIndex={hasFilters ? undefined : -1} className={buttonClass({ variant: 'ghost', className: `focus-ring ${hasFilters ? '' : 'invisible'}` })}>
          Clear
        </button>
        <div className="hidden w-24 shrink-0 border-l border-line px-3 text-right sm:block" aria-busy={isSearching || undefined}>
          {isSearching || (!countKnown && !error)
            ? <div className="flex h-5 items-center justify-end"><span className="skeleton h-3.5 w-10 rounded" /><span className="sr-only">Count not known yet</span></div>
            : !countKnown
              ? <div className="flex h-5 items-center justify-end text-caption text-muted">Unavailable</div>
              : <div className="text-heading tabular-nums text-fg">{formatMatchCount(resultCount)}</div>}
          <div className="text-overline uppercase text-muted">Matches</div>
        </div>
      </div>
      <div className="mt-1.5 flex min-h-6 flex-wrap items-center gap-2 px-2 text-caption">
        <Switch checked={interpretPlainLanguage} onCheckedChange={setInterpretPlainLanguage} className="whitespace-nowrap text-caption">
          Plain-language search
        </Switch>
        {parsedQuery.filters.map((filter, index) => {
          const deferred = deferredFilterKeys.has(filterKey(filter));
          return (
          <span key={`${filter.field}-${filter.value}-${index}`} title={deferred ? 'Recognized qualifier, but not applied to this search' : `Applied qualifier: ${filter.raw ?? serializeQualifierQuery({ version: 1, source: 'qualifier', text: '', filters: [filter], issues: [] })}`} className={deferred ? 'rounded-full border border-warning/30 bg-warning/10 px-2 py-0.5 font-semibold text-warning' : 'rounded-full border border-accent/30 bg-accent/5 px-2 py-0.5 font-semibold text-accent'}>
            {filter.negated ? '-' : ''}{filter.field}:{filter.value}
          </span>
          );
        })}
        {compiledQuery.deferredFilters.length > 0 && (
          <span className="text-warning" role="status">
            {compiledQuery.deferredFilters.length} qualifier{compiledQuery.deferredFilters.length === 1 ? '' : 's'} not applied
          </span>
        )}
        {parsedQuery.issues.some((issue) => issue.severity === 'warning' || issue.severity === 'error') && (
          <span className="text-warning" title="Some query tokens need review">Query syntax needs review</span>
        )}
        {selectedPolicyDeviation && (
          <button type="button" onClick={() => setSearchFilters(inputValue, selectedAgencyId, selectedStatus, null)} className="focus-ring inline-flex h-6 items-center rounded-full border border-warning/30 bg-warning/10 px-2 font-medium text-warning">
            {formatPolicy(selectedPolicyDeviation)} ×
          </button>
        )}
        <button type="button" onClick={() => void copyRecord()} disabled={isSearching} className="focus-ring inline-flex h-6 items-center whitespace-nowrap rounded-full border border-control px-2 font-medium text-fg-secondary hover:bg-raised disabled:opacity-50 ml-auto">
          Copy search record
        </button>
        {canWatch && (
          <button type="button" onClick={() => void watchTerm()} title="Watches this text across all sources. The agency, status and policy filters are not part of a watch." className="focus-ring inline-flex h-6 items-center whitespace-nowrap rounded-full border border-control px-2 font-medium text-fg-secondary hover:bg-raised disabled:opacity-50">
            Watch this text for changes
          </button>
        )}
        {canStand && (
          <button type="button" onClick={() => void saveStanding()} disabled={isSearching} title="Saves this exact query, filters included. It re-runs whenever the corpus changes and records the corpus state each run saw." className="focus-ring inline-flex h-6 items-center whitespace-nowrap rounded-full border border-control px-2 font-medium text-fg-secondary hover:bg-raised disabled:opacity-50">
            Save as standing query
          </button>
        )}
        {watchStatus && (
          <span role={watchStatus.ok ? 'status' : 'alert'} className={watchStatus.ok ? 'text-success' : 'text-danger'}>
            {watchStatus.text}
            {watchStatus.ok && <button type="button" onClick={() => setWorkspaceView('overview')} className="ml-1 underline">Open watchlist</button>}
          </span>
        )}
        {/* With nothing retained, the workbench states the failure itself. */}
        {error && retainedCount > 0 && (
          <span role="alert" className="basis-full text-danger">
            Search unavailable; showing {formatMatchCount(retainedCount)} results from the last successful query: {error}
          </span>
        )}
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
