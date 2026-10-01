import { useCallback, useEffect, useMemo, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useInspectFinding } from '../../hooks/useInspectFinding';
import { useAppStore } from '../../store/useAppStore';
import { KIND_LABELS } from '../tracker/MyRequestsPanel';
import type { Finding } from '../../types';
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
const SOURCES: TimelineSource[] = ['REQUEST', 'FINDING', 'WATCH'];

interface SourceState {
  items: TimelineItem[];
  error: string | null;
  loaded: boolean;
  capped: boolean;
}
const EMPTY: SourceState = { items: [], error: null, loaded: false, capped: false };

/** Loads one source independently, so one failing source is named as missing instead of blanking the others. */
function useSource(load: () => Promise<{ items: TimelineItem[]; capped: boolean }>) {
  const gate = useLatestRequestGate();
  const [state, setState] = useState<SourceState>(EMPTY);
  useEffect(() => {
    const request = gate.begin();
    load()
      .then((result) => {
        if (gate.isCurrent(request)) setState({ ...result, error: null, loaded: true });
      })
      .catch((reason) => {
        if (gate.isCurrent(request)) setState({ items: [], capped: false, loaded: true, error: formatNativeError(reason) });
      });
  }, [gate, load]);
  return state;
}

export default function CaseTimeline() {
  const findingsRefresh = useAppStore((state) => state.findingsRefreshRequests);
  const setWorkspaceView = useAppStore((state) => state.setWorkspaceView);
  const inspectFinding = useInspectFinding();
  const [findingsById, setFindingsById] = useState<Map<string, Finding>>(new Map());
  const [shown, setShown] = useState<ReadonlySet<TimelineSource>>(new Set(SOURCES));
  const [from, setFrom] = useState('');
  const [to, setTo] = useState('');
  const [text, setText] = useState('');
  const [limit, setLimit] = useState(PAGE);
  const [expanded, setExpanded] = useState<string | null>(null);

  const loadRequests = useCallback(async () => {
    if (!isTauri()) return { items: [], capped: false };
    return { items: requestItems(await invokeCommand('list_claimed_requests', {}), KIND_LABELS), capped: false };
  }, []);
  // The refresh counter re-keys the loader so a completed ingest re-reads findings.
  const loadFindings = useCallback(async () => {
    void findingsRefresh;
    if (!isTauri()) return { items: [], capped: false };
    const rows = await invokeCommand('list_findings', { filter: null, limit: FINDING_LIMIT + 1 });
    const kept = rows.slice(0, FINDING_LIMIT);
    setFindingsById(new Map(kept.map((finding) => [finding.id, finding])));
    return { items: findingItems(kept), capped: rows.length > FINDING_LIMIT };
  }, [findingsRefresh]);
  const loadWatch = useCallback(async () => {
    if (!isTauri()) return { items: [], capped: false };
    const rows = await invokeCommand('list_watch_changes', { watchId: null, limit: WATCH_LIMIT + 1 });
    return { items: watchItems(rows.slice(0, WATCH_LIMIT)), capped: rows.length > WATCH_LIMIT };
  }, []);

  const sources: Record<TimelineSource, SourceState> = {
    REQUEST: useSource(loadRequests),
    FINDING: useSource(loadFindings),
    WATCH: useSource(loadWatch),
  };

  const all = useMemo(() => SOURCES.flatMap((source) => sources[source].items), [sources.REQUEST.items, sources.FINDING.items, sources.WATCH.items]);
  const view = useMemo(() => buildTimeline(all, { sources: shown, from, to, text }), [all, shown, from, to, text]);
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
        <p className="text-[10px] leading-relaxed text-slate-400">
          One timeline from your tracked requests, detector findings and watch changes. An event date is when something happened. Last seen and noticed dates are when this app saw it, which can be later than the event.
        </p>

        <div className="app-panel flex flex-wrap items-center gap-2 p-2 text-[10px] text-slate-300">
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
                className={`focus-ring rounded-md border px-2 py-1 font-semibold ${shown.has(source) ? 'border-cyan-400/50 text-cyan-200' : 'border-slate-800 text-slate-400'}`}
              >
                {SOURCE_LABELS[source]}
              </button>
            ))}
          </div>
          <label className="flex items-center gap-1">From<input type="date" value={from} max={to || undefined} onChange={(event) => { setFrom(event.target.value); setLimit(PAGE); }} className="focus-ring rounded border border-slate-800 bg-slate-950 px-1 py-0.5" /></label>
          <label className="flex items-center gap-1">To<input type="date" value={to} min={from || undefined} onChange={(event) => { setTo(event.target.value); setLimit(PAGE); }} className="focus-ring rounded border border-slate-800 bg-slate-950 px-1 py-0.5" /></label>
          <label className="sr-only" htmlFor="timeline-text">Filter timeline by text</label>
          <input id="timeline-text" type="search" value={text} onChange={(event) => { setText(event.target.value); setLimit(PAGE); }} placeholder="Filter timeline" className="focus-ring min-w-[10rem] flex-1 rounded border border-slate-800 bg-slate-950 px-2 py-1" />
        </div>

        {SOURCES.filter((source) => sources[source].error).map((source) => (
          <p key={source} role="alert" className="rounded-lg border border-rose-500/20 bg-rose-500/[0.04] px-3 py-2 text-[10px] text-rose-200">
            {SOURCE_LABELS[source]} could not be read, so they are missing from this timeline: {sources[source].error}
          </p>
        ))}
        {SOURCES.filter((source) => sources[source].capped).map((source) => (
          <p key={source} role="status" className="text-[10px] text-amber-200">
            Only the most recent {source === 'FINDING' ? FINDING_LIMIT : WATCH_LIMIT} {SOURCE_LABELS[source].toLowerCase()} are loaded; older ones are not shown.
          </p>
        ))}
        {view.unreadableDates > 0 && (
          <p role="status" className="text-[10px] text-amber-200">{view.unreadableDates} item{view.unreadableDates === 1 ? ' has' : 's have'} a date that could not be read and {view.unreadableDates === 1 ? 'is' : 'are'} not shown.</p>
        )}

        {loading && all.length === 0 ? (
          <p role="status" className="text-[10px] text-slate-400">Loading the timeline…</p>
        ) : view.items.length === 0 ? (
          <div className="app-panel p-5 text-center text-[10px] text-slate-400">
            {all.length === 0
              ? 'Nothing to place on a timeline yet. Dates you record in My requests, findings and watch changes appear here. This is not a statement that nothing happened.'
              : 'No items match these filters.'}
          </div>
        ) : (
          groups.map((group) => (
            <section key={group.month} aria-label={`Items from ${group.month}`}>
              <h3 className="mb-1 text-[11px] font-semibold text-slate-200">{group.month}</h3>
              <ol className="divide-y divide-slate-800/70 rounded-xl border border-slate-800">
                {group.rows.map((row) => (
                  <li key={row.id} className="px-3 py-2">
                    <div className="flex flex-wrap items-baseline gap-x-2 text-[10px]">
                      <time dateTime={row.at} className="font-mono text-slate-300">{row.at.slice(0, 10)}</time>
                      <span className="rounded border border-slate-700 px-1.5 text-[8px] uppercase tracking-wider text-slate-400">{DATE_MEANING_LABELS[row.dateMeaning]}</span>
                      <span className="rounded border border-slate-700 px-1.5 text-[8px] uppercase tracking-wider text-slate-400">{SOURCE_LABELS[row.source]}</span>
                    </div>
                    <p className="mt-1 text-[11px] font-medium text-slate-100">{row.title}</p>
                    {row.detail && <p className="mt-0.5 text-[10px] text-slate-400">{row.detail}</p>}
                    <p className="mt-0.5 text-[9px] text-slate-400">{row.provenance}</p>
                    <div className="mt-1 flex gap-3 text-[9px]">
                      {row.evidence && (
                        <button type="button" aria-expanded={expanded === row.id} onClick={() => setExpanded((current) => (current === row.id ? null : row.id))} className="focus-ring underline text-slate-300">
                          {expanded === row.id ? 'Hide basis' : 'Show basis'}
                        </button>
                      )}
                      {row.findingId && findingsById.get(row.findingId) && (
                        <button type="button" onClick={() => inspectFinding(findingsById.get(row.findingId as string) as Finding)} className="focus-ring underline text-slate-300">Inspect in evidence</button>
                      )}
                      {row.source === 'WATCH' && (
                        <button type="button" onClick={() => setWorkspaceView('overview')} className="focus-ring underline text-slate-300">Open watchlist</button>
                      )}
                    </div>
                    {expanded === row.id && row.evidence && <blockquote className="mt-1 border-l border-slate-600 pl-2 text-[10px] text-slate-300">{row.evidence}</blockquote>}
                  </li>
                ))}
              </ol>
            </section>
          ))
        )}
        {view.items.length > limit && (
          <button type="button" onClick={() => setLimit((current) => current + PAGE)} className="secondary-action focus-ring rounded-lg border px-3 py-1.5 text-[10px] font-semibold text-slate-300">
            Show {PAGE} more ({view.items.length - limit} remaining)
          </button>
        )}
      </div>
    </div>
  );
}
