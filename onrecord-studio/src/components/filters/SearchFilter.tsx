import { formatMatchCount, SEARCH_PAGE_SIZE } from '../../utils/searchResults';
import type { SearchPageArgs } from '../../types';
import { useEffect, useMemo, useRef, useState } from 'react';
import AgencySelector from './AgencySelector';
import { useAppStore } from '../../store/useAppStore';
import { searchQueryId, searchRecordText, type SearchRecordInput } from '../../utils/searchResults';
import { formatNativeError, invokeCommand } from '../../ipc';
import { compileToLegacyQuery, interpretNaturalLanguage, parseQualifierQuery, serializeQualifierQuery } from '../../query';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import Button from '../ui/Button';
import Badge from '../ui/Badge';
import { SearchField, Select } from '../ui/Field';

/** `onPendingChange` reports whether the latest search is still running, so the results pane can
 * tell "not answered yet" from "answered with nothing". */
export default function SearchFilter({ onPendingChange }: { onPendingChange?: (pending: boolean) => void }) {
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
    setIsSearching(true);
    onPendingChange?.(true);
    const timer = window.setTimeout(async () => {
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
        if (requestGate.isCurrent(request)) {
          setIsSearching(false);
          onPendingChange?.(false);
        }
      }
    }, 220);
    return () => window.clearTimeout(timer);
  }, [activePortalKey, compiledQuery, evidenceRefreshRequests, inputValue, requestGate, selectedAgencyId, selectedPolicyDeviation, selectedStatus, setFilteredResults, setSearchError, onPendingChange]);

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
    <section aria-label="Evidence search" className="shrink-0 px-5 pt-6 lg:px-6">
      <div className="flex flex-wrap items-center gap-2">
        <SearchField
          id="evidence-search"
          aria-label="Search requests and extracted documents"
          value={inputValue}
          onChange={(event) => updateTerm(event.target.value)}
          placeholder="Search request metadata and extracted text"
          wrapperClassName="min-w-[240px] flex-1"
        />
        <AgencySelector />
        <Select
          aria-label="Request status"
          value={selectedStatus ?? ''}
          onChange={(event) => setSearchFilters(inputValue, selectedAgencyId, event.target.value || null, selectedPolicyDeviation)}
          className="w-36"
        >
          <option value="">All statuses</option>
          <option value="open">Open</option>
          <option value="closed">Closed</option>
          <option value="referred">Referred</option>
        </Select>
        {hasFilters && <Button variant="ghost" onClick={clear}>Clear</Button>}
        {/* A failed or running search has no count; showing the last one, or 0, would be a claim. */}
        <p className="ml-auto min-w-[6.5rem] shrink-0 text-right text-body text-muted" aria-live="polite">
          {isSearching ? 'Searching…' : error ? <><span aria-hidden="true">—</span> matches</> : (
            <><span className="tabular-nums font-medium text-fg">{formatMatchCount(resultCount)}</span> {resultCount === 1 ? 'match' : 'matches'}</>
          )}
          {error && !isSearching && <span className="sr-only">Match count unavailable</span>}
        </p>
      </div>
      <div className="-ml-3 mt-2 flex min-h-7 flex-wrap items-center gap-1">
        <Button
          variant="ghost"
          size="sm"
          aria-pressed={interpretPlainLanguage}
          onClick={() => setInterpretPlainLanguage((enabled) => !enabled)}
          className={interpretPlainLanguage ? 'bg-accent/10 text-accent hover:bg-accent/15 hover:text-accent' : undefined}
        >
          Plain-language search
        </Button>
        <Button variant="ghost" size="sm" onClick={() => void copyRecord()} disabled={isSearching}>
          Copy search record
        </Button>
        {canWatch && (
          <Button variant="ghost" size="sm" onClick={() => void watchTerm()} title="Watches this text across all sources. The agency, status and policy filters are not part of a watch.">
            Watch this text
          </Button>
        )}
        {canStand && (
          <Button variant="ghost" size="sm" onClick={() => void saveStanding()} disabled={isSearching} title="Saves this exact query, filters included. It re-runs whenever the corpus changes and records the corpus state each run saw.">
            Save as standing query
          </Button>
        )}
        {parsedQuery.filters.map((filter, index) => {
          const deferred = deferredFilterKeys.has(filterKey(filter));
          return (
            <Badge
              key={`${filter.field}-${filter.value}-${index}`}
              tone={deferred ? 'warning' : 'neutral'}
              title={deferred ? 'Recognized qualifier, but not applied to this search' : `Applied qualifier: ${filter.raw ?? serializeQualifierQuery({ version: 1, source: 'qualifier', text: '', filters: [filter], issues: [] })}`}
            >
              {filter.negated ? '-' : ''}{filter.field}:{filter.value}
            </Badge>
          );
        })}
        {compiledQuery.deferredFilters.length > 0 && (
          <span className="text-caption text-warning" role="status">
            <span className="tabular-nums">{compiledQuery.deferredFilters.length}</span> qualifier{compiledQuery.deferredFilters.length === 1 ? '' : 's'} not applied
          </span>
        )}
        {parsedQuery.issues.some((issue) => issue.severity === 'warning' || issue.severity === 'error') && (
          <span className="text-caption text-warning" title="Some query tokens need review">Query syntax needs review</span>
        )}
        {selectedPolicyDeviation && (
          <Button variant="ghost" size="sm" onClick={() => setSearchFilters(inputValue, selectedAgencyId, selectedStatus, null)} aria-label={`Remove the ${formatPolicy(selectedPolicyDeviation)} filter`} className="text-warning hover:text-warning">
            {formatPolicy(selectedPolicyDeviation)} <span aria-hidden="true">×</span>
          </Button>
        )}
        {watchStatus && (
          <span role={watchStatus.ok ? 'status' : 'alert'} className={watchStatus.ok ? 'text-caption text-success' : 'text-caption text-danger'}>
            {watchStatus.text}
            {watchStatus.ok && <Button variant="link" onClick={() => setWorkspaceView('overview')} className="ml-1 text-caption">Open watchlist</Button>}
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
