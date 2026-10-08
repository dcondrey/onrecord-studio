import { useCallback, useEffect, useRef, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import { openHelp } from '../help/helpEvents';
import type { SeriesGapReport, StandingQueryFilters, StandingQueryOutcome, StandingQuerySummary } from '../../types/standingSearch';
import { describeCountBasis, describeGap, describeRun, formatFacetCount } from '../../utils/standingSearch';
import MethodNote from '../ui/MethodNote';
import Panel, { PanelHeader } from '../ui/Panel';
import Button from '../ui/Button';
import Badge from '../ui/Badge';
import InlineError from '../ui/InlineError';
import Skeleton from '../ui/Skeleton';
import Time from '../ui/Time';
import { formatInstant } from '../objects/labels';

const FACET_LABELS: Record<string, string> = { status: 'Status', agency: 'Department', portal: 'Source', year: 'Year filed' };

/** The executed query in full: the text plus every flat filter, including ones chosen by dropdown. */
function describeFilters(filters: StandingQueryFilters): string {
  return [
    filters.searchTerm.trim() ? `“${filters.searchTerm.trim()}”` : null,
    filters.status ? `status:${filters.status}` : null,
    filters.policyDeviation ? `flag:${filters.policyDeviation}` : null,
    filters.agencyId !== null ? `agency #${filters.agencyId}` : null,
    filters.portalKey ? `source:${filters.portalKey}` : null,
  ].filter((part): part is string => part !== null).join(' ') || '(every request)';
}

/**
 * Saved searches re-run against each new corpus state, shown as breakdowns and clusters before
 * any row, and the series-number absence search. Nothing here is a row list: open a query in the
 * evidence search to read records.
 */
export default function StandingSearchPanel() {
  const refreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
  const setActivePortal = useAppStore((state) => state.setActivePortal);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const [summaries, setSummaries] = useState<StandingQuerySummary[] | null>(null);
  const [gaps, setGaps] = useState<SeriesGapReport | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const gate = useLatestRequestGate();
  const lastRunRequest = useRef(0);

  const sync = useCallback(
    async (runDue: boolean) => {
      const request = gate.begin();
      setBusy(true);
      setError(null);
      try {
        let outcomes: StandingQueryOutcome[] = [];
        if (runDue) outcomes = await invokeCommand('run_standing_queries', { queryId: null, force: false });
        const [list, report] = await Promise.all([
          invokeCommand('list_standing_queries', {}),
          invokeCommand('list_series_number_gaps', { limit: 50 }),
        ]);
        if (!gate.isCurrent(request)) return;
        if (!Array.isArray(list)) throw new Error('The native standing query list was invalid.');
        if (!report || !Array.isArray(report.gaps)) throw new Error('The native series gap report was invalid.');
        setSummaries(list);
        setGaps(report);
        const deferred = outcomes.filter((outcome) => outcome.outcome === 'DEFERRED');
        const ran = outcomes.filter((outcome) => outcome.outcome === 'RAN').length;
        setNotice(
          runDue
            ? `${ran} ${ran === 1 ? 'query' : 'queries'} re-run${deferred.length > 0 ? `; ${deferred.length} deferred: ${deferred[0].outcome === 'DEFERRED' ? deferred[0].reason : ''}` : ''}.`
            : null,
        );
      } catch (reason) {
        if (!gate.isCurrent(request)) return;
        setError(formatNativeError(reason));
      } finally {
        if (gate.isCurrent(request)) setBusy(false);
      }
    },
    [gate],
  );

  useEffect(() => {
    const runDue = refreshRequests > 0 && refreshRequests !== lastRunRequest.current;
    lastRunRequest.current = refreshRequests;
    void sync(runDue);
    return gate.invalidate;
  }, [refreshRequests, sync, gate]);

  const remove = async (queryId: string) => {
    setError(null);
    try {
      await invokeCommand('delete_standing_query', { queryId });
      await sync(false);
    } catch (reason) {
      setError(formatNativeError(reason));
    }
  };

  const open = (summary: StandingQuerySummary) => {
    const { searchTerm, agencyId, status, policyDeviation, portalKey } = summary.query.filters;
    setActivePortal(portalKey);
    setSearchFilters(searchTerm, agencyId, status, policyDeviation);
    setWorkspaceView('evidence');
  };

  return (
    <Panel aria-label="Standing searches">
      <PanelHeader
        title="Standing searches"
        description={<>
          Saved queries re-run whenever the corpus changes. Each run names the corpus state it saw.{' '}
          <Button variant="link" className="text-caption" onClick={() => openHelp('standing-search')}>How to read a run</Button>
        </>}
        actions={<Button disabled={busy} onClick={() => void sync(true)}>{busy ? 'Running…' : 'Run due queries'}</Button>}
      />
      <div className="space-y-4 p-5 text-body">
      {notice && <p role="status" className="text-caption text-muted">{notice}</p>}
      {error && <InlineError summary="Standing searches couldn’t be read." detail={error} onRetry={() => void sync(false)} />}

      {summaries === null ? (
        !error && <Skeleton variant="lines" lines={2} label="Reading standing searches" />
      ) : summaries.length === 0 ? (
        <p className="text-muted">None saved. Run a search in the evidence view and choose “Save as standing query”.</p>
      ) : (
        <ul className="space-y-3">
          {summaries.map((summary) => (
            <li key={summary.query.id} className="rounded-lg border border-line p-3">
              <div className="flex flex-wrap items-center gap-2">
                <span className="font-medium text-fg">{summary.query.label || 'Untitled search'}</span>
                <span className="min-w-0 truncate text-caption text-muted">{describeFilters(summary.query.filters)}</span>
                {summary.due && <Badge tone="warning">Corpus changed since last run</Badge>}
                <span className="ml-auto flex gap-1">
                  <Button size="sm" variant="ghost" onClick={() => open(summary)}>Open in evidence</Button>
                  <Button size="sm" variant="ghost" className="text-danger hover:text-danger" onClick={() => void remove(summary.query.id)}>Delete</Button>
                </span>
              </div>
              {summary.lastRun ? (
                <RunView run={summary.lastRun} />
              ) : (
                <p className="mt-1 text-caption text-muted">Never run.</p>
              )}
            </li>
          ))}
        </ul>
      )}

      <div className="border-t border-line pt-4">
        <h4 className="text-body font-medium text-fg">Missing series numbers</h4>
        <div className="mt-1 text-caption text-muted">
          Gaps in numbered series of resolutions, ordinances, contracts and permits.{' '}
          <MethodNote>
            A gap is a number absent from the corpus between two documents of one labeled series. It is a lead for a request,
            not evidence that the agency withheld anything.
          </MethodNote>
        </div>
        {gaps === null ? null : gaps.gaps.length === 0 ? (
          <p className="mt-2 text-caption text-muted">
            {gaps.seriesCount === 0 ? 'No labeled series with three or more numbers has been read from the corpus.' : `No short gaps in ${gaps.seriesCount} series.`}
          </p>
        ) : (
          <ul className="mt-2 space-y-1 text-body text-fg-secondary">
            {gaps.gaps.map((gap) => (
              <li key={`${gap.label}:${gap.prefix}:${gap.fromNumber}`}>{describeGap(gap)}</li>
            ))}
            {gaps.truncated && <li className="text-muted">More gaps exist than are listed.</li>}
          </ul>
        )}
      </div>
      </div>
    </Panel>
  );
}

function RunView({ run }: { run: StandingQuerySummary['lastRun'] & object }) {
  const basis = run.result.countBasis;
  return (
    <div className="mt-2 space-y-2 text-fg-secondary">
      <p className="text-caption tabular-nums text-muted" title={describeRun(run)}>
        Ran <Time value={run.ranAt} format={formatInstant} className="whitespace-nowrap" /> · {run.result.matchCount.toLocaleString()} matching request{run.result.matchCount === 1 ? '' : 's'}
        {' · '}{run.changedSincePrevious === null ? 'first run' : run.changedSincePrevious ? `match set changed from ${run.previousMatchCount?.toLocaleString() ?? 'an unrecorded count'} to ${run.result.matchCount.toLocaleString()}` : 'no change since the previous run'}
      </p>
      <p className="text-caption text-muted">Breakdowns below are {describeCountBasis(basis)}.</p>
      <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
        {run.result.facets.map((facet) => (
          <dl key={facet.dimension} className="rounded-md bg-raised/60 p-2 text-caption">
            <dt className="font-medium text-fg">{FACET_LABELS[facet.dimension] ?? facet.dimension}</dt>
            {facet.values.map((value) => (
              <dd key={value.value} className="flex justify-between gap-2"><span className="truncate">{value.value || '(none)'}</span><span className="tabular-nums">{formatFacetCount(value.count, basis)}</span></dd>
            ))}
            {facet.truncated && <dd className="text-muted">More values not listed.</dd>}
          </dl>
        ))}
      </div>
      {run.result.clusters.length > 0 && (
        <div>
          <p className="font-medium text-fg">Identical documents across requests</p>
          <ul className="mt-1 space-y-0.5 text-caption">
            {run.result.clusters.map((cluster) => (
              <li key={cluster.extractedTextSha256}>
                {cluster.exampleFileName || 'Document'}: identical extracted text under {cluster.requestCount} requests ({cluster.documentCount} copies) <span className="font-mono text-code text-muted" title={cluster.extractedTextSha256}>sha256:{cluster.extractedTextSha256.slice(0, 16)}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
