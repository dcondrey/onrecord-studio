import { useCallback, useEffect, useRef, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import type { DimensionResult, WatchChange, WatchDimension, WatchKind, WatchSummary } from '../../types';
import MethodNote from '../ui/MethodNote';
import Panel, { PanelHeader } from '../ui/Panel';
import Button from '../ui/Button';
import InlineError from '../ui/InlineError';
import Skeleton from '../ui/Skeleton';
import Time from '../ui/Time';
import { Field, Input, Select } from '../ui/Field';
import { formatDateTime, formatShortDate } from '../objects/labels';
import { cn } from '../../utils/cn';

const KIND_LABELS: Record<WatchKind, string> = {
  AGENCY: 'Agency',
  ENTITY: 'Person or entity',
  TOPIC_PACK: 'Topic pack',
  QUERY: 'Search query',
  SOURCE: 'Document source',
};

const KIND_HINTS: Record<WatchKind, string> = {
  AGENCY: 'Agency name as the portal records it',
  ENTITY: 'Name to find in document text',
  TOPIC_PACK: 'Topic id, for example abatement',
  QUERY: 'Up to 8 words; every word must appear',
  SOURCE: 'Source key, for example the portal subdomain',
};

const DIMENSION_LABELS: Record<WatchDimension, string> = {
  DOCUMENTS: 'Documents',
  FINDINGS: 'Findings',
  FIGURES: 'Figures',
  WITHHELD: 'Redaction',
  SOURCE_HEALTH: 'Source',
  SOURCE_CHECKS: 'Source copies',
};

const CHANGE_LABELS: Record<string, string> = {
  DOCUMENT_ADDED: 'New document',
  DOCUMENT_NO_LONGER_LISTED: 'Document no longer listed',
  FINDING_ADDED: 'New finding',
  FINDING_RESOLVED: 'Finding no longer emitted',
  FIGURE_CHANGED: 'Figure changed',
  FIGURE_NO_LONGER_STATED: 'Figure no longer stated',
  FIGURE_ADDED: 'New figure',
  NEWLY_REDACTED: 'Newly flagged as heavily redacted',
  REDACTION_FLAG_NO_LONGER_RECORDED: 'Redaction flag no longer recorded',
  SOURCE_STOPPED_RESPONDING: 'Source stopped responding',
  SOURCE_RESPONDING_AGAIN: 'Source responding again',
};

/**
 * One line of status for a dimension. "No data this run" and "no change" are different facts and
 * are worded and coloured differently: only `UNCHANGED` may say nothing changed.
 */
export function describeDimension(result: DimensionResult): { text: string; tone: 'quiet' | 'changed' | 'missing' } {
  switch (result.outcome) {
    case 'NOT_MEASURED':
      return { text: `No data this run: ${result.reason ?? 'the dimension could not be observed'}`, tone: 'missing' };
    case 'BASELINE_RECORDED':
      return { text: `Baseline recorded${result.observedAsOf ? ` ${formatShortDate(result.observedAsOf)}` : ''}; nothing to compare yet`, tone: 'quiet' };
    case 'CHANGED':
      return {
        text: `${result.changeCount} ${result.changeCount === 1 ? 'change' : 'changes'} since ${result.baselineAsOf ? formatShortDate(result.baselineAsOf) : 'the baseline'}`,
        tone: 'changed',
      };
    case 'UNCHANGED':
      return { text: `Observed, no change since ${result.baselineAsOf ? formatShortDate(result.baselineAsOf) : 'the baseline'}`, tone: 'quiet' };
  }
}

function asList<T>(value: T[] | undefined, what: string): T[] {
  if (!Array.isArray(value)) throw new Error(`The native ${what} response was invalid.`);
  return value;
}

/**
 * Saved watches and what changed for each since its last check. A check reads the local corpus
 * only. Each change states the old value, the new value, when each was observed, and the basis.
 */
export default function WatchlistPanel() {
  const refreshRequests = useAppStore((state) => state.findingsRefreshRequests);
  const [summaries, setSummaries] = useState<WatchSummary[]>([]);
  const [changes, setChanges] = useState<WatchChange[]>([]);
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [reload, setReload] = useState(0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [kind, setKind] = useState<WatchKind>('AGENCY');
  const [target, setTarget] = useState('');
  const gate = useLatestRequestGate();
  const lastCheckedRequest = useRef(0);

  // Loads the list and the changes for one watch filter. The filter is the identity this request
  // was made under: a slower answer for a watch the viewer has since left must not be shown as
  // the answer for the watch now selected.
  const sync = useCallback(
    async (watchId: string | null, check: boolean) => {
      const request = gate.begin();
      setBusy(true);
      setError(null);
      try {
        if (check) await invokeCommand('check_watches', { watchId: null });
        const [list, rows] = await Promise.all([
          invokeCommand('list_watches', {}),
          invokeCommand('list_watch_changes', { watchId, limit: 100 }),
        ]);
        if (!gate.isCurrent(request)) return;
        setSummaries(asList(list, 'watch list'));
        setChanges(asList(rows, 'change list'));
        setLoaded(true);
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
    const check = refreshRequests > 0 && refreshRequests !== lastCheckedRequest.current;
    lastCheckedRequest.current = refreshRequests;
    void sync(selectedId, check);
    return gate.invalidate;
  }, [selectedId, refreshRequests, reload, sync, gate]);

  const add = async () => {
    const text = target.trim();
    if (!text) return;
    setError(null);
    try {
      const saved = await invokeCommand('save_watch', {
        watch: { id: '', kind, target: text, label: '', createdAt: '', enabled: true },
      });
      setTarget('');
      setSelectedId(saved.id);
    } catch (reason) {
      setError(formatNativeError(reason));
    }
  };

  const remove = async (id: string) => {
    setError(null);
    try {
      await invokeCommand('delete_watch', { watchId: id });
      // The viewer may have selected another watch while the delete was pending, so the selection
      // is read at resolution time and the effect reloads for whatever is selected now.
      setSelectedId((current) => (current === id ? null : current));
      setReload((count) => count + 1);
    } catch (reason) {
      setError(formatNativeError(reason));
    }
  };

  const selected = summaries.find((summary) => summary.watch.id === selectedId) ?? null;

  return (
    <Panel aria-label="Watchlist">
      <PanelHeader
        title="Watchlist"
        description={<>
          Checks the records already stored here; nothing is fetched.{' '}
          <MethodNote>
            The first check of a watch records a baseline and reports no changes. A dimension with no data in a run says so and is
            never shown as unchanged.
          </MethodNote>
        </>}
        actions={<Button disabled={busy || summaries.length === 0} onClick={() => void sync(selectedId, true)}>{busy ? 'Checking…' : 'Check now'}</Button>}
      />
      <div className="space-y-4 p-5 text-body text-fg-secondary">
      {error && <InlineError summary="The watchlist couldn’t be updated." detail={error} />}
      <form
        className="grid items-end gap-3 sm:grid-cols-[12rem_minmax(0,1fr)_auto]"
        onSubmit={(event) => {
          event.preventDefault();
          void add();
        }}
      >
        <Field label="Watch">
          <Select value={kind} onChange={(event) => setKind(event.target.value as WatchKind)}>
            {(Object.keys(KIND_LABELS) as WatchKind[]).map((value) => (
              <option key={value} value={value}>
                {KIND_LABELS[value]}
              </option>
            ))}
          </Select>
        </Field>
        <Field label="Name or identifier">
          <Input value={target} onChange={(event) => setTarget(event.target.value)} placeholder={KIND_HINTS[kind]} maxLength={200} />
        </Field>
        <Button type="submit" disabled={!target.trim()}>Add to watchlist</Button>
      </form>

      {!loaded && !error && <Skeleton variant="lines" lines={3} label="Reading the watchlist" />}
      {loaded && summaries.length === 0 && <p className="text-muted">Nothing is being watched yet.</p>}
      <ul className="space-y-2">
        {summaries.map(({ watch, lastCheckedAt, lastDimensions }) => (
          <li key={watch.id} className={cn('rounded-lg border p-3', watch.id === selectedId ? 'border-accent/30 bg-accent/5' : 'border-line')}>
            <div className="flex flex-wrap items-center justify-between gap-2">
              <button type="button" className="focus-ring rounded text-left font-medium text-fg hover:underline" aria-pressed={watch.id === selectedId} onClick={() => setSelectedId(watch.id === selectedId ? null : watch.id)}>
                {watch.label} <span className="font-normal text-muted">· {KIND_LABELS[watch.kind]}</span>
              </button>
              <Button size="sm" variant="ghost" aria-label={`Stop watching ${watch.label}`} onClick={() => void remove(watch.id)}>
                Remove
              </Button>
            </div>
            <p className="mt-1 text-caption text-muted">{lastCheckedAt ? <>Last checked <Time value={lastCheckedAt} format={formatDateTime} className="whitespace-nowrap" /></> : 'Not checked yet'}</p>
            {lastDimensions.length > 0 && (
              <ul className="mt-2 space-y-0.5 text-caption">
                {lastDimensions.map((result) => {
                  const { text, tone } = describeDimension(result);
                  return (
                    <li key={result.dimension} className={tone === 'missing' ? 'text-warning' : tone === 'changed' ? 'text-fg' : 'text-muted'}>
                      <span className="font-medium">{DIMENSION_LABELS[result.dimension]}:</span> {text}
                    </li>
                  );
                })}
              </ul>
            )}
          </li>
        ))}
      </ul>

      {loaded && (
        <div aria-label="Recorded changes">
          <h4 className="text-body font-medium text-fg">{selected ? `Changes for ${selected.watch.label}` : 'Changes across all watches'}</h4>
          {changes.length === 0 ? (
            <p className="mt-1 text-muted">No changes recorded. This lists only changes that were detected; see each watch above for what a check could and could not observe.</p>
          ) : (
            <ul className="mt-1 space-y-2">
              {changes.map((change) => (
                <li key={change.id} className="rounded-lg border border-line p-3">
                  <div className="font-medium text-fg">{CHANGE_LABELS[change.changeKind] ?? change.changeKind}</div>
                  <div className="text-muted">{change.subject}</div>
                  <dl className="mt-1 grid gap-0.5 sm:grid-cols-2">
                    <div>
                      <dt className="text-caption text-muted">Before{change.oldAsOf ? `, as of ${formatShortDate(change.oldAsOf)}` : ''}</dt>
                      <dd>{change.oldValue ?? 'Not present at the baseline'}</dd>
                    </div>
                    <div>
                      <dt className="text-caption text-muted">Now, as of {formatShortDate(change.newAsOf)}</dt>
                      <dd>{change.newValue ?? 'Not present now'}</dd>
                    </div>
                  </dl>
                  <p className="mt-1 text-caption text-muted">Basis: {change.basis}</p>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
      </div>
    </Panel>
  );
}
