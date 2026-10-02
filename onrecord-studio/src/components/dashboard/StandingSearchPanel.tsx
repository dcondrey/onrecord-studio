import { useCallback, useEffect, useRef, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import { openHelp } from '../help/helpEvents';
import type { SeriesGapReport, StandingQueryOutcome, StandingQuerySummary } from '../../types/standingSearch';
import { describeCountBasis, describeGap, describeRun, formatFacetCount } from '../../utils/standingSearch';

const FACET_LABELS: Record<string, string> = { status: 'Status', agency: 'Department', portal: 'Source', year: 'Year filed' };

/**
 * Saved searches re-run against each new corpus state, shown as breakdowns and clusters before
 * any row, and the series-number absence search. Nothing here is a row list: open a query in the
 * evidence search to read records.
 */
export default function StandingSearchPanel() {
  const refreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const setSearchFilters = useAppStore((state) => state.setSearchFilters);
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
    const { searchTerm, agencyId, status, policyDeviation } = summary.query.filters;
    setSearchFilters(searchTerm, agencyId, status, policyDeviation);
    setWorkspaceView('evidence');
  };

  return (
    <section className="app-panel p-4" aria-label="Standing searches">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <div>
          <div className="app-eyebrow">Standing searches</div>
          <p className="mt-1 text-label leading-4 text-slate-400">
            Saved queries re-run whenever the corpus changes. Each run names the corpus state it saw.{' '}
            <button type="button" className="underline" onClick={() => openHelp('standing-search')}>How to read a run</button>
          </p>
        </div>
        <button type="button" className="analysis-button" disabled={busy} onClick={() => void sync(true)}>{busy ? 'Working…' : 'Run due queries'}</button>
      </div>
      {notice && <p role="status" className="mt-2 text-label text-slate-400">{notice}</p>}
      {error && <p role="alert" className="mt-2 text-label text-amber-200">{error}</p>}

      {summaries === null ? (
        <p className="mt-3 text-label text-slate-400">Reading standing searches.</p>
      ) : summaries.length === 0 ? (
        <p className="mt-3 text-label text-slate-400">None saved. Run a search in the evidence view and choose “Save as standing query”.</p>
      ) : (
        <ul className="mt-3 space-y-3">
          {summaries.map((summary) => (
            <li key={summary.query.id} className="rounded-lg border border-slate-800 p-3 text-label">
              <div className="flex flex-wrap items-center gap-2">
                <span className="font-semibold text-slate-100">{summary.query.label || '(untitled)'}</span>
                <span className="font-mono text-slate-400">{JSON.parse(summary.query.astJson).text || ''}</span>
                {summary.due && <span className="rounded-full border border-amber-500/40 px-2 py-0.5 text-amber-200">Corpus changed since last run</span>}
                <span className="ml-auto flex gap-2">
                  <button type="button" className="underline" onClick={() => open(summary)}>Open rows in evidence search</button>
                  <button type="button" className="underline text-rose-300" onClick={() => void remove(summary.query.id)}>Delete</button>
                </span>
              </div>
              {summary.lastRun ? (
                <RunView run={summary.lastRun} />
              ) : (
                <p className="mt-1 text-slate-400">Never run.</p>
              )}
            </li>
          ))}
        </ul>
      )}

      <div className="mt-4 border-t border-slate-800 pt-3">
        <div className="app-eyebrow">Missing series numbers</div>
        <p className="mt-1 text-label text-slate-400">
          Numbers absent from the corpus between two documents of one labelled series (resolutions, ordinances, contracts, permits). A gap is a lead for a request, not evidence that the agency withheld anything.
        </p>
        {gaps === null ? null : gaps.gaps.length === 0 ? (
          <p className="mt-2 text-label text-slate-400">
            {gaps.seriesCount === 0 ? 'No labelled series with three or more numbers has been read from the corpus.' : `No short gaps in ${gaps.seriesCount} series.`}
          </p>
        ) : (
          <ul className="mt-2 space-y-1 text-label text-slate-300">
            {gaps.gaps.map((gap) => (
              <li key={`${gap.label}:${gap.prefix}:${gap.fromNumber}`}>{describeGap(gap)}</li>
            ))}
            {gaps.truncated && <li className="text-slate-400">More gaps exist than are listed.</li>}
          </ul>
        )}
      </div>
    </section>
  );
}

function RunView({ run }: { run: StandingQuerySummary['lastRun'] & object }) {
  const basis = run.result.countBasis;
  return (
    <div className="mt-1 space-y-2 text-slate-300">
      <p>{describeRun(run)}</p>
      <p className="text-slate-400">Breakdowns below are {describeCountBasis(basis)}.</p>
      <div className="grid gap-2 sm:grid-cols-2 lg:grid-cols-4">
        {run.result.facets.map((facet) => (
          <dl key={facet.dimension} className="rounded border border-slate-800 p-2">
            <dt className="font-semibold text-slate-200">{FACET_LABELS[facet.dimension] ?? facet.dimension}</dt>
            {facet.values.map((value) => (
              <dd key={value.value} className="flex justify-between gap-2"><span className="truncate">{value.value || '(none)'}</span><span className="font-mono">{formatFacetCount(value.count, basis)}</span></dd>
            ))}
            {facet.truncated && <dd className="text-slate-400">More values not listed.</dd>}
          </dl>
        ))}
      </div>
      {run.result.clusters.length > 0 && (
        <div>
          <p className="font-semibold text-slate-200">Identical documents across requests</p>
          <ul className="mt-1 space-y-0.5">
            {run.result.clusters.map((cluster) => (
              <li key={cluster.extractedTextSha256}>
                {cluster.exampleFileName || 'Document'}: identical extracted text under {cluster.requestCount} requests ({cluster.documentCount} copies), sha256:{cluster.extractedTextSha256.slice(0, 16)}
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
