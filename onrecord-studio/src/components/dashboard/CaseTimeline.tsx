import { useCallback, useEffect, useMemo, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useInspectFinding } from '../../hooks/useInspectFinding';
import { useAppStore } from '../../store/useAppStore';
import { KIND_LABELS } from '../tracker/MyRequestsPanel';
import { cn } from '../../utils/cn';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import EmptyState from '../ui/EmptyState';
import InlineError from '../ui/InlineError';
import { Input, SearchField, Select } from '../ui/Field';
import MethodNote from '../ui/MethodNote';
import Skeleton from '../ui/Skeleton';
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
  // Bumped by Retry; every loader reads it so each re-runs under its own request gate.
  const [reload, setReload] = useState(0);

  const loadRequests = useCallback(async (): Promise<Loaded<null>> => {
    void reload;
    if (!isTauri()) return NOTHING;
    return { items: requestItems(await invokeCommand('list_claimed_requests', {}), KIND_LABELS), capped: false, detail: null };
  }, [reload]);
  // The refresh counter re-keys the loader so a completed ingest re-reads findings.
  const loadFindings = useCallback(async (): Promise<Loaded<Map<string, Finding>>> => {
    void findingsRefresh;
    void reload;
    if (!isTauri()) return NOTHING;
    const rows = await invokeCommand('list_findings', { filter: null, limit: FINDING_LIMIT + 1 });
    const kept = rows.slice(0, FINDING_LIMIT);
    return { items: findingItems(kept), capped: rows.length > FINDING_LIMIT, detail: new Map(kept.map((finding) => [finding.id, finding])) };
  }, [findingsRefresh, reload]);
  const loadWatch = useCallback(async (): Promise<Loaded<null>> => {
    void reload;
    if (!isTauri()) return NOTHING;
    const rows = await invokeCommand('list_watch_changes', { watchId: null, limit: WATCH_LIMIT + 1 });
    return { items: watchItems(rows.slice(0, WATCH_LIMIT)), capped: rows.length > WATCH_LIMIT, detail: null };
  }, [reload]);
  const loadCase = useCallback(async (): Promise<Loaded<{ lanes: TimelineLane[]; contradictions: DateContradiction[] }>> => {
    void reload;
    if (!isTauri()) return NOTHING;
    const built = caseItems((await invokeCommand('get_casework_state', {})).cases);
    return { items: built.items, capped: false, detail: { lanes: built.lanes, contradictions: built.contradictions } };
  }, [reload]);

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
  const pending = SOURCES.filter((source) => !sources[source].loaded);
  const failed = SOURCES.filter((source) => sources[source].error);

  const groups: { month: string; rows: TimelineItem[] }[] = [];
  for (const row of visible) {
    const month = monthKey(row.at);
    const last = groups[groups.length - 1];
    if (last && last.month === month) last.rows.push(row);
    else groups.push({ month, rows: [row] });
  }

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="w-full max-w-[1100px] space-y-4 px-5 pb-10 pt-4 lg:px-6">
        <div className="app-panel grid gap-3 p-3">
          <div className="flex flex-wrap items-center gap-2">
            <div role="group" aria-label="Show sources" className="flex flex-wrap items-center gap-1.5">
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
                  className={cn(
                    'focus-ring inline-flex h-8 items-center gap-1.5 whitespace-nowrap rounded-lg border px-2.5 text-control transition-colors',
                    shown.has(source) ? 'border-accent/30 bg-accent/10 text-accent' : 'border-line-strong text-muted hover:text-fg',
                  )}
                >
                  {/* A tick for on, an empty box for off, so the state never rests on the fill alone. */}
                  <svg viewBox="0 0 12 12" className="h-3 w-3 shrink-0" fill="none" stroke="currentColor" strokeWidth="1.5" aria-hidden="true">
                    <rect x="0.75" y="0.75" width="10.5" height="10.5" rx="2.5" />
                    {shown.has(source) && <path d="M3.5 6.2 5.2 7.9 8.6 4.3" />}
                  </svg>
                  {SOURCE_LABELS[source]}
                </button>
              ))}
            </div>
            <SearchField
              value={text}
              onChange={(event) => { setText(event.target.value); setLimit(PAGE); }}
              placeholder="Filter timeline"
              aria-label="Filter timeline by text"
              wrapperClassName="min-w-[12rem] flex-1"
            />
          </div>
          <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
            <label className="flex items-center gap-2 text-caption text-muted">From
              <Input type="date" value={from} max={to || undefined} onChange={(event) => { setFrom(event.target.value); setLimit(PAGE); }} className="w-40 tabular-nums" />
            </label>
            <label className="flex items-center gap-2 text-caption text-muted">To
              <Input type="date" value={to} min={from || undefined} onChange={(event) => { setTo(event.target.value); setLimit(PAGE); }} className="w-40 tabular-nums" />
            </label>
            <label className="flex items-center gap-2 text-caption text-muted">Lane
              <Select value={lane ?? ''} disabled={caseLanes.length === 0} onChange={(event) => { setLane(event.target.value || null); setLimit(PAGE); }} className="w-44">
                {!casework.loaded ? <option value="">Loading lanes…</option>
                  : casework.error ? <option value="">Lanes not available</option>
                    : caseLanes.length === 0 ? <option value="">No lanes</option>
                      : <option value="">All lanes</option>}
                {caseLanes.map((option) => <option key={option.id} value={option.id}>{option.label}</option>)}
              </Select>
            </label>
          </div>
        </div>

        {failed.length > 0 && (
          <InlineError
            summary={`${listOf(failed.map((source) => SOURCE_LABELS[source]))} could not be read, so ${failed.length === 1 ? 'it is' : 'they are'} missing from this timeline.`}
            detail={failed.map((source) => `${SOURCE_LABELS[source]}: ${sources[source].error}`).join(' · ')}
            onRetry={() => setReload((count) => count + 1)}
          />
        )}
        {SOURCES.filter((source) => sources[source].capped).map((source) => (
          <p key={source} role="status" className="text-caption text-warning">
            Only the most recent {source === 'FINDING' ? FINDING_LIMIT : WATCH_LIMIT} {SOURCE_LABELS[source].toLowerCase()} are loaded; older ones are not shown.
          </p>
        ))}
        {contradictions.length > 0 && (
          <section aria-label="Date contradictions" className="rounded-lg border border-warning/30 bg-warning/10 px-4 py-3 text-body text-warning">
            <h3 className="text-heading">Two sources give different dates</h3>
            <ul className="mt-1 space-y-1">
              {contradictions.map((entry) => (
                <li key={entry.id}>
                  <span className="font-medium">{entry.subject}</span>
                  <ul className="ml-4 list-disc">
                    {entry.readings.map((reading) => <li key={reading.date}><time dateTime={reading.date} className="tabular-nums">{formatDay(reading.date)}</time>: {reading.basis}</li>)}
                  </ul>
                </li>
              ))}
            </ul>
            <p className="mt-1">Both readings are shown as recorded. Resolving which is right needs a sourced record, not a choice here.</p>
          </section>
        )}
        {view.unreadableDates > 0 && (
          <p role="status" className="text-caption text-warning">{view.unreadableDates} item{view.unreadableDates === 1 ? ' has' : 's have'} a date that could not be read and {view.unreadableDates === 1 ? 'is' : 'are'} not shown.</p>
        )}

        {pending.length > 0 && all.length > 0 && (
          <p role="status" className="text-caption text-muted">Still loading {listOf(pending.map((source) => SOURCE_LABELS[source].toLowerCase()))}…</p>
        )}
        {pending.length > 0 && all.length === 0 ? (
          <div className="space-y-2">
            <Skeleton variant="block" height={88} label="Loading the timeline" />
            {[1, 2].map((slot) => <div key={slot} aria-hidden="true"><Skeleton variant="block" height={88} label="" /></div>)}
          </div>
        ) : view.items.length === 0 ? (
          all.length === 0 ? (
            <EmptyState
              icon="calendar"
              title="Nothing on the timeline yet"
              body="Dates appear here when you track a request, a detector finds something, or a watch changes. An empty timeline does not mean nothing happened."
            />
          ) : (
            <p className="app-panel p-5 text-center text-body text-muted">No items match these filters.</p>
          )
        ) : (
          groups.map((group, index) => (
            <section key={group.month} aria-label={`Items from ${formatMonth(group.month)}`}>
              <div className="mb-2 flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
                <h3 className="text-heading text-fg">{formatMonth(group.month)}</h3>
                {index === 0 && (
                  <MethodNote label="How dates are read">
                    One timeline from your tracked requests, detector findings, watch changes and case records. An event date is when something happened. Last seen and noticed dates are when this app saw it, which can be later than the event. A due or modeled date is owed or computed, not observed. Case dates are the ones you recorded; none is read out of a document.
                  </MethodNote>
                )}
              </div>
              <ol className="app-panel divide-y divide-line overflow-hidden">
                {group.rows.map((row) => (
                  <li key={row.id} className="px-4 py-3">
                    <div className="flex flex-wrap items-center gap-2">
                      <time dateTime={row.at} className="text-caption font-medium tabular-nums text-fg-secondary">{formatDay(row.at)}</time>
                      <Badge tone="neutral">{DATE_MEANING_LABELS[row.dateMeaning]}</Badge>
                      <Badge tone="neutral">{SOURCE_LABELS[row.source]}</Badge>
                    </div>
                    <p className="mt-1.5 text-control text-fg">{row.title}</p>
                    {row.detail && <p className="mt-0.5 text-body text-muted">{row.detail}</p>}
                    <p className="mt-0.5 text-caption text-muted">{row.provenance}</p>
                    {row.citations && row.citations.length === 0 && <p className="mt-0.5 text-caption text-warning">No citation recorded: this date rests on your entry alone.</p>}
                    {(row.evidence || (row.findingId && findingsById?.get(row.findingId)) || row.source === 'WATCH') && (
                      <div className="mt-2 flex flex-wrap gap-2">
                        {row.evidence && (
                          <Button size="sm" variant="link" className="px-0" aria-expanded={expanded === row.id} onClick={() => setExpanded((current) => (current === row.id ? null : row.id))}>
                            {expanded === row.id ? 'Hide basis' : 'Show basis'}
                          </Button>
                        )}
                        {row.findingId && findingsById?.get(row.findingId) && (
                          <Button size="sm" onClick={() => inspectFinding(findingsById.get(row.findingId as string) as Finding)}>Open finding</Button>
                        )}
                        {row.source === 'WATCH' && (
                          <Button size="sm" onClick={() => setWorkspaceView('overview')}>Open watchlist</Button>
                        )}
                      </div>
                    )}
                    {expanded === row.id && row.evidence && <blockquote className="mt-2 border-l-2 border-line-strong pl-3 text-body text-fg-secondary">{row.evidence}</blockquote>}
                  </li>
                ))}
              </ol>
            </section>
          ))
        )}
        {view.items.length > limit && (
          <Button onClick={() => setLimit((current) => current + PAGE)}>
            Show {PAGE} more ({(view.items.length - limit).toLocaleString()} remaining)
          </Button>
        )}
      </div>
    </div>
  );
}

const MONTH = new Intl.DateTimeFormat(undefined, { month: 'long', year: 'numeric', timeZone: 'UTC' });

/** "2026-10" → "October 2026". The key is a calendar month, so it is read in UTC, not shifted by zone. */
function formatMonth(key: string): string {
  const [year, month] = key.split('-').map(Number);
  return Number.isFinite(year) && Number.isFinite(month) ? MONTH.format(new Date(Date.UTC(year, month - 1, 1))) : key;
}

/** A bare date is a calendar day and is shown as written; an instant is shown in local time. */
function formatDay(at: string): string {
  const parsed = new Date(at);
  if (Number.isNaN(parsed.getTime())) return at;
  return new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', year: 'numeric', timeZone: at.length <= 10 ? 'UTC' : undefined }).format(parsed);
}

function listOf(parts: string[]): string {
  if (parts.length <= 1) return parts.join('');
  return `${parts.slice(0, -1).join(', ')} and ${parts[parts.length - 1]}`;
}
