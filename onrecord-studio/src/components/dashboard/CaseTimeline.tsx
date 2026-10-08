import { useCallback, useEffect, useMemo, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useInspectFinding } from '../../hooks/useInspectFinding';
import { useAppStore } from '../../store/useAppStore';
import { KIND_LABELS } from '../tracker/MyRequestsPanel';
import type { Finding } from '../../types';
import { caseItems, type DateContradiction, type TimelineLane } from '../../utils/caseTimeline';
import {
  buildTimeline,
  DATE_MEANING_LABELS,
  findingItems,
  monthKey,
  requestItems,
  SOURCE_LABELS,
  watchItems,
  type TimelineItem,
  type TimelineSource,
} from '../../utils/timeline';

const FINDING_LIMIT = 500;
const WATCH_LIMIT = 200;
const PAGE = 100;
const SOURCES: TimelineSource[] = ['REQUEST', 'FINDING', 'WATCH', 'CASE'];

interface Loaded<D> {
  items: TimelineItem[];
  capped: boolean;
  /** Data the view needs beside the items; published under the same gate as them. */
  detail: D | null;
}
interface SourceState<D> extends Loaded<D> {
  error: string | null;
  loaded: boolean;
}

/** Loads one source independently, so one failing source is named as missing instead of blanking the others. */
function useSource<D>(load: () => Promise<Loaded<D>>) {
  const gate = useLatestRequestGate();
  const [state, setState] = useState<SourceState<D>>({ items: [], capped: false, detail: null, error: null, loaded: false });
  useEffect(() => {
    const request = gate.begin();
    load()
      .then((result) => {
        if (gate.isCurrent(request)) setState({ ...result, error: null, loaded: true });
      })
      .catch((reason) => {
        if (gate.isCurrent(request)) setState({ items: [], capped: false, detail: null, loaded: true, error: formatNativeError(reason) });
      });
  }, [gate, load]);
  return state;
}

const NOTHING = { items: [], capped: false, detail: null };

export default function CaseTimeline() {
  const findingsRefresh = useAppStore((state) => state.findingsRefreshRequests);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const inspectFinding = useInspectFinding();
  const [shown, setShown] = useState<ReadonlySet<TimelineSource>>(new Set(SOURCES));
  const [from, setFrom] = useState('');
  const [to, setTo] = useState('');
  const [text, setText] = useState('');
  const [limit, setLimit] = useState(PAGE);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [lane, setLane] = useState<string | null>(null);

  const loadRequests = useCallback(async (): Promise<Loaded<null>> => {
    if (!isTauri()) return NOTHING;
    return { items: requestItems(await invokeCommand('list_claimed_requests', {}), KIND_LABELS), capped: false, detail: null };
  }, []);
  // The refresh counter re-keys the loader so a completed ingest re-reads findings.
  const loadFindings = useCallback(async (): Promise<Loaded<Map<string, Finding>>> => {
    void findingsRefresh;
    if (!isTauri()) return NOTHING;
    const rows = await invokeCommand('list_findings', { filter: null, limit: FINDING_LIMIT + 1 });
    const kept = rows.slice(0, FINDING_LIMIT);
    return { items: findingItems(kept), capped: rows.length > FINDING_LIMIT, detail: new Map(kept.map((finding) => [finding.id, finding])) };
  }, [findingsRefresh]);
  const loadWatch = useCallback(async (): Promise<Loaded<null>> => {
    if (!isTauri()) return NOTHING;
    const rows = await invokeCommand('list_watch_changes', { watchId: null, limit: WATCH_LIMIT + 1 });
    return { items: watchItems(rows.slice(0, WATCH_LIMIT)), capped: rows.length > WATCH_LIMIT, detail: null };
  }, []);
  const loadCase = useCallback(async (): Promise<Loaded<{ lanes: TimelineLane[]; contradictions: DateContradiction[] }>> => {
    if (!isTauri()) return NOTHING;
    const built = caseItems((await invokeCommand('get_casework_state', {})).cases);
    return { items: built.items, capped: false, detail: { lanes: built.lanes, contradictions: built.contradictions } };
  }, []);

  const findings = useSource(loadFindings);
  const casework = useSource(loadCase);
  const sources: Record<TimelineSource, SourceState<unknown>> = {
    REQUEST: useSource(loadRequests),
    FINDING: findings,
    WATCH: useSource(loadWatch),
    CASE: casework,
  };
  const findingsById = findings.detail;
  const caseLanes = casework.detail?.lanes ?? [];
  const contradictions = casework.detail?.contradictions ?? [];

  const all = useMemo(() => SOURCES.flatMap((source) => sources[source].items), [sources.REQUEST.items, sources.FINDING.items, sources.WATCH.items, sources.CASE.items]);
  const view = useMemo(() => buildTimeline(all, { sources: shown, from, to, text, lane }), [all, shown, from, to, text, lane]);
  const visible = view.items.slice(0, limit);
  const loading = SOURCES.some((source) => !sources[source].loaded);

  const groups: { month: string; rows: TimelineItem[] }[] = [];
  for (const row of visible) {
    const month = monthKey(row.at);
    const last = groups[groups.length - 1];
    if (last && last.month === month) last.rows.push(row);
    else groups.push({ month, rows: [row] });
  }

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-[1100px] space-y-3 p-4 lg:p-5">
        <p className="text-label leading-relaxed text-muted">
          One timeline from your tracked requests, detector findings, watch changes and case records. An event date is when something happened. Last seen and noticed dates are when this app saw it, which can be later than the event. A due or modeled date is owed or computed, not observed. Case dates are the ones you recorded; none is read out of a document.
        </p>

        <div className="app-panel flex flex-wrap items-center gap-2 p-2 text-label text-fg-secondary">
          <div role="group" aria-label="Show sources" className="flex gap-1">
            {SOURCES.map((source) => (
              <button
                key={source}
                type="button"
                aria-pressed={shown.has(source)}
                onClick={() =>
                  setShown((current) => {
                    const next = new Set(current);
                    if (next.has(source)) next.delete(source);
                    else next.add(source);
                    return next;
                  })
                }
                className={`focus-ring rounded-md border px-2 py-1 font-semibold ${shown.has(source) ? 'border-accent/60 text-accent' : 'border-control text-muted'}`}
              >
                {SOURCE_LABELS[source]}
              </button>
            ))}
          </div>
          <label className="flex items-center gap-1">From<input type="date" value={from} max={to || undefined} onChange={(event) => { setFrom(event.target.value); setLimit(PAGE); }} className="focus-ring rounded border border-control bg-canvas px-1 py-0.5" /></label>
          <label className="flex items-center gap-1">To<input type="date" value={to} min={from || undefined} onChange={(event) => { setTo(event.target.value); setLimit(PAGE); }} className="focus-ring rounded border border-control bg-canvas px-1 py-0.5" /></label>
          {caseLanes.length > 0 && (
            <label className="flex items-center gap-1">Lane
              <select value={lane ?? ''} onChange={(event) => { setLane(event.target.value || null); setLimit(PAGE); }} className="focus-ring rounded border border-control bg-canvas px-1 py-0.5">
                <option value="">All lanes</option>
                {caseLanes.map((option) => <option key={option.id} value={option.id}>{option.label}</option>)}
              </select>
            </label>
          )}
          <label className="sr-only" htmlFor="timeline-text">Filter timeline by text</label>
          <input id="timeline-text" type="search" value={text} onChange={(event) => { setText(event.target.value); setLimit(PAGE); }} placeholder="Filter timeline" className="focus-ring min-w-[10rem] flex-1 rounded border border-control bg-canvas px-2 py-1" />
        </div>

        {SOURCES.filter((source) => sources[source].error).map((source) => (
          <p key={source} role="alert" className="rounded-lg border border-danger/30 bg-danger/5 px-3 py-2 text-label text-danger">
            {SOURCE_LABELS[source]} could not be read, so they are missing from this timeline: {sources[source].error}
          </p>
        ))}
        {SOURCES.filter((source) => sources[source].capped).map((source) => (
          <p key={source} role="status" className="text-label text-warning">
            Only the most recent {source === 'FINDING' ? FINDING_LIMIT : WATCH_LIMIT} {SOURCE_LABELS[source].toLowerCase()} are loaded; older ones are not shown.
          </p>
        ))}
        {contradictions.length > 0 && (
          <section aria-label="Date contradictions" className="rounded-lg border border-warning/30 bg-warning/5 px-3 py-2 text-label text-warning">
            <h3 className="font-semibold">Two sources give different dates</h3>
            <ul className="mt-1 space-y-1">
              {contradictions.map((entry) => (
                <li key={entry.id}>
                  <span className="font-medium">{entry.subject}</span>
                  <ul className="ml-3 list-disc">
                    {entry.readings.map((reading) => <li key={reading.date}><time dateTime={reading.date} className="font-mono">{reading.date}</time>: {reading.basis}</li>)}
                  </ul>
                </li>
              ))}
            </ul>
            <p className="mt-1 text-warning">Both readings are shown as recorded. Resolving which is right needs a sourced record, not a choice here.</p>
          </section>
        )}
        {view.unreadableDates > 0 && (
          <p role="status" className="text-label text-warning">{view.unreadableDates} item{view.unreadableDates === 1 ? ' has' : 's have'} a date that could not be read and {view.unreadableDates === 1 ? 'is' : 'are'} not shown.</p>
        )}

        {loading && all.length === 0 ? (
          <p role="status" className="text-label text-muted">Loading the timeline…</p>
        ) : view.items.length === 0 ? (
          <div className="app-panel p-5 text-center text-label text-muted">
            {all.length === 0
              ? 'Nothing to place on a timeline yet. Dates you record in My requests, findings and watch changes appear here. This is not a statement that nothing happened.'
              : 'No items match these filters.'}
          </div>
        ) : (
          groups.map((group) => (
            <section key={group.month} aria-label={`Items from ${group.month}`}>
              <h3 className="mb-1 text-label font-semibold text-fg">{group.month}</h3>
              <ol className="divide-y divide-line rounded-xl border border-line">
                {group.rows.map((row) => (
                  <li key={row.id} className="px-3 py-2">
                    <div className="flex flex-wrap items-baseline gap-x-2 text-label">
                      <time dateTime={row.at} className="font-mono text-fg-secondary">{row.at.slice(0, 10)}</time>
                      <span className="rounded border border-line-strong px-1.5 text-caption uppercase tracking-wider text-muted">{DATE_MEANING_LABELS[row.dateMeaning]}</span>
                      <span className="rounded border border-line-strong px-1.5 text-caption uppercase tracking-wider text-muted">{SOURCE_LABELS[row.source]}</span>
                    </div>
                    <p className="mt-1 text-label font-medium text-fg">{row.title}</p>
                    {row.detail && <p className="mt-0.5 text-label text-muted">{row.detail}</p>}
                    <p className="mt-0.5 text-caption text-muted">{row.provenance}</p>
                    {row.citations && row.citations.length === 0 && <p className="mt-0.5 text-caption text-warning">No citation recorded: this date rests on your entry alone.</p>}
                    <div className="mt-1 flex gap-3 text-caption">
                      {row.evidence && (
                        <button type="button" aria-expanded={expanded === row.id} onClick={() => setExpanded((current) => (current === row.id ? null : row.id))} className="focus-ring underline text-fg-secondary">
                          {expanded === row.id ? 'Hide basis' : 'Show basis'}
                        </button>
                      )}
                      {row.findingId && findingsById?.get(row.findingId) && (
                        <button type="button" onClick={() => inspectFinding(findingsById.get(row.findingId as string) as Finding)} className="focus-ring underline text-fg-secondary">Inspect in evidence</button>
                      )}
                      {row.source === 'WATCH' && (
                        <button type="button" onClick={() => setWorkspaceView('overview')} className="focus-ring underline text-fg-secondary">Open watchlist</button>
                      )}
                    </div>
                    {expanded === row.id && row.evidence && <blockquote className="mt-1 border-l border-line-strong pl-2 text-label text-fg-secondary">{row.evidence}</blockquote>}
                  </li>
                ))}
              </ol>
            </section>
          ))
        )}
        {view.items.length > limit && (
          <button type="button" onClick={() => setLimit((current) => current + PAGE)} className="secondary-action focus-ring rounded-lg border px-3 py-1.5 text-label font-semibold text-fg-secondary">
            Show {PAGE} more ({view.items.length - limit} remaining)
          </button>
        )}
      </div>
    </div>
  );
}
