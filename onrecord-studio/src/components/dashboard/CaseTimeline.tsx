import { useCallback, useEffect, useMemo, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { invokeCommand } from '../../ipc';
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
import { displayError } from '../../utils/errorMessage';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import InlineError from '../ui/InlineError';
import Skeleton from '../ui/Skeleton';
import Time from '../ui/Time';
import { Input, SearchField, Select } from '../ui/Field';
import { cn } from '../../utils/cn';

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
  error: { message: string; detail: string } | null;
  loaded: boolean;
}

/** Loads one source independently, so one failing source is named as missing instead of blanking the others. */
function useSource<D>(load: () => Promise<Loaded<D>>): SourceState<D> & { retry: () => void } {
  const gate = useLatestRequestGate();
  const [attempt, setAttempt] = useState(0);
  const [state, setState] = useState<SourceState<D>>({ items: [], capped: false, detail: null, error: null, loaded: false });
  useEffect(() => {
    const request = gate.begin();
    if (attempt > 0) setState((current) => ({ ...current, error: null, loaded: false }));
    load()
      .then((result) => {
        if (gate.isCurrent(request)) setState({ ...result, error: null, loaded: true });
      })
      .catch((reason) => {
        if (!gate.isCurrent(request)) return;
        const { message, detail } = displayError(reason);
        setState({ items: [], capped: false, detail: null, loaded: true, error: { message, detail } });
      });
  }, [attempt, gate, load]);
  const retry = useCallback(() => setAttempt((count) => count + 1), []);
  return { ...state, retry };
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
  const sources: Record<TimelineSource, SourceState<unknown> & { retry: () => void }> = {
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
    const month = monthLabel(monthKey(row.at));
    const last = groups[groups.length - 1];
    if (last && last.month === month) last.rows.push(row);
    else groups.push({ month, rows: [row] });
  }

  return (
    <div className="min-h-0 flex-1 overflow-y-auto">
      <div className="mx-auto w-full max-w-[1100px] space-y-4 px-6 pb-6 pt-4">
        <p className="max-w-prose text-body text-muted">
          One timeline from your tracked requests, detector findings, watch changes and case records. An event date is when something happened. Last seen and noticed dates are when this app saw it, which can be later than the event. A due or modeled date is owed or computed, not observed. Case dates are the ones you recorded; none is read out of a document.
        </p>

        <div className="app-panel flex flex-wrap items-center gap-2 p-2 text-control text-fg-secondary">
          <div role="group" aria-label="Show sources" className="flex flex-wrap gap-1">
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
                className={cn('focus-ring h-8 rounded-lg border px-3 text-control transition-colors', shown.has(source) ? 'border-accent/30 bg-accent/10 text-accent' : 'border-line-strong text-muted hover:bg-raised hover:text-fg')}
              >
                {SOURCE_LABELS[source]}
              </button>
            ))}
          </div>
          <label className="flex items-center gap-1.5 text-muted">From<Input type="date" value={from} max={to || undefined} onChange={(event) => { setFrom(event.target.value); setLimit(PAGE); }} className="focus-ring w-[9.5rem] tabular-nums" /></label>
          <label className="flex items-center gap-1.5 text-muted">To<Input type="date" value={to} min={from || undefined} onChange={(event) => { setTo(event.target.value); setLimit(PAGE); }} className="focus-ring w-[9.5rem] tabular-nums" /></label>
          {caseLanes.length > 0 && (
            <Select aria-label="Case lane" value={lane ?? ''} onChange={(event) => { setLane(event.target.value || null); setLimit(PAGE); }} className="focus-ring w-40">
              <option value="">All lanes</option>
              {caseLanes.map((option) => <option key={option.id} value={option.id}>{option.label}</option>)}
            </Select>
          )}
          <SearchField id="timeline-text" aria-label="Filter timeline by text" value={text} onChange={(event) => { setText(event.target.value); setLimit(PAGE); }} placeholder="Filter timeline" wrapperClassName="min-w-[10rem] flex-1" />
        </div>

        {SOURCES.map((source) => {
          const error = sources[source].error;
          return error && (
            <InlineError
              key={source}
              summary={`${SOURCE_LABELS[source]} could not be read, so they are missing from this timeline. ${error.message}`}
              detail={error.detail}
              onRetry={sources[source].retry}
            />
          );
        })}
        {/* A lane still reading is named, so a partial timeline never reads as a complete one. */}
        {all.length > 0 && SOURCES.some((source) => !sources[source].loaded) && (
          <p role="status" className="flex items-center gap-2 text-caption text-muted">
            <span aria-hidden="true" className="skeleton h-2 w-8 rounded" />
            Still reading {SOURCES.filter((source) => !sources[source].loaded).map((source) => SOURCE_LABELS[source].toLowerCase()).join(', ')}; they are not on this timeline yet.
          </p>
        )}
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
                    {entry.readings.map((reading) => <li key={reading.date}><Time value={reading.date} />: {reading.basis}</li>)}
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
          <Skeleton variant="lines" lines={6} label="Loading the timeline" className="app-panel p-5" />
        ) : view.items.length === 0 ? (
          <div className="app-panel p-5 text-center text-body text-muted">
            {all.length === 0
              ? 'Nothing to place on a timeline yet. Dates you record in My requests, findings and watch changes appear here. This is not a statement that nothing happened.'
              : 'No items match these filters.'}
          </div>
        ) : (
          groups.map((group) => (
            <section key={group.month} aria-label={`Items from ${group.month}`}>
              <h3 className="mb-2 text-heading text-fg">{group.month}</h3>
              <ol className="divide-y divide-line overflow-hidden rounded-xl border border-line bg-panel shadow-panel">
                {group.rows.map((row) => (
                  <li key={row.id} className="px-4 py-3">
                    <div className="flex flex-wrap items-center gap-2 text-caption">
                      <Time value={row.at.length >= 10 ? row.at.slice(0, 10) : row.at} className="font-medium text-fg-secondary" />
                      <Badge>{DATE_MEANING_LABELS[row.dateMeaning]}</Badge>
                      <Badge>{SOURCE_LABELS[row.source]}</Badge>
                    </div>
                    <p className="mt-1 text-body font-medium text-fg">{row.title}</p>
                    {row.detail && <p className="mt-0.5 text-body text-muted">{row.detail}</p>}
                    <p className="mt-0.5 text-caption text-muted">{row.provenance}</p>
                    {row.citations && row.citations.length === 0 && <p className="mt-0.5 text-caption text-warning">No citation recorded: this date rests on your entry alone.</p>}
                    <div className="mt-1 flex gap-3 text-caption [&>button]:text-accent [&>button]:underline-offset-2 hover:[&>button]:underline">
                      {row.evidence && (
                        <button type="button" aria-expanded={expanded === row.id} onClick={() => setExpanded((current) => (current === row.id ? null : row.id))} className="focus-ring rounded">
                          {expanded === row.id ? 'Hide basis' : 'Show basis'}
                        </button>
                      )}
                      {row.findingId && findingsById?.get(row.findingId) && (
                        <button type="button" onClick={() => inspectFinding(findingsById.get(row.findingId as string) as Finding)} className="focus-ring rounded">Inspect in evidence</button>
                      )}
                      {row.source === 'WATCH' && (
                        <button type="button" onClick={() => setWorkspaceView('overview')} className="focus-ring rounded">Open watchlist</button>
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
          <Button onClick={() => setLimit((current) => current + PAGE)}>
            Show {PAGE} more ({(view.items.length - limit).toLocaleString('en-US')} remaining)
          </Button>
        )}
      </div>
    </div>
  );
}

/** "2026-10" as "October 2026"; a key that is not a month is shown as recorded. */
function monthLabel(key: string): string {
  const [year, month] = key.split('-').map(Number);
  if (!year || !month || month < 1 || month > 12) return key;
  return new Intl.DateTimeFormat(undefined, { month: 'long', year: 'numeric', timeZone: 'UTC' }).format(new Date(Date.UTC(year, month - 1, 1)));
}
