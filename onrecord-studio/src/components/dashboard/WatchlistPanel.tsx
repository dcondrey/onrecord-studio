import { useCallback, useEffect, useRef, useState } from 'react';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore } from '../../store/useAppStore';
import type { DimensionResult, WatchChange, WatchDimension, WatchKind, WatchSummary } from '../../types';
import MethodNote from '../ui/MethodNote';
import { Field, Input, Select } from '../ui/Field';
import { formatDateTime, formatShortDate } from '../objects/labels';

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
      return { text: `Baseline recorded${result.observedAsOf ? ` ${result.observedAsOf}` : ''}; nothing to compare yet`, tone: 'quiet' };
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
    <section aria-label="Watchlist" className="app-panel space-y-3 p-4 text-label text-fg-secondary">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <h3 className="text-heading text-fg">What changed for the things you follow</h3>
        <button type="button" className="analysis-button" disabled={busy || summaries.length === 0} onClick={() => void sync(selectedId, true)}>
          {busy ? 'Checking…' : 'Check now'}
        </button>
      </div>
      <div className="text-muted">
        Checks the records already stored here; nothing is fetched.{' '}
        <MethodNote>
          The first check of a watch records a baseline and reports no changes. A dimension with no data in a run says so and is
          never shown as unchanged.
        </MethodNote>
      </div>
      {error && (
        <p role="alert" className="text-danger">
          Watchlist: {error}
        </p>
      )}
      <form
        className="flex flex-wrap items-end gap-2"
        onSubmit={(event) => {
          event.preventDefault();
          void add();
        }}
      >
        <Select label="Watch" value={kind} onChange={(event) => setKind(event.target.value as WatchKind)}>
          {(Object.keys(KIND_LABELS) as WatchKind[]).map((value) => (
            <option key={value} value={value}>
              {KIND_LABELS[value]}
            </option>
          ))}
        </Select>
        <Field label="Target" className="min-w-[14rem] flex-1">
          <Input value={target} onChange={(event) => setTarget(event.target.value)} placeholder={KIND_HINTS[kind]} maxLength={200} />
        </Field>
        <button type="submit" className="analysis-button" disabled={!target.trim()}>
          Add to watchlist
        </button>
      </form>

      {!loaded && !error && <p role="status">Reading the watchlist…</p>}
      {loaded && summaries.length === 0 && <p className="text-muted">Nothing is being watched yet.</p>}
      <ul className="space-y-2">
        {summaries.map(({ watch, lastCheckedAt, lastDimensions }) => (
          <li key={watch.id} className={`rounded-lg border p-2 ${watch.id === selectedId ? 'border-accent/60' : 'border-line'}`}>
            <div className="flex flex-wrap items-center justify-between gap-2">
              <button type="button" className="text-left font-semibold text-fg" aria-pressed={watch.id === selectedId} onClick={() => setSelectedId(watch.id === selectedId ? null : watch.id)}>
                {watch.label} <span className="font-normal text-muted">({KIND_LABELS[watch.kind]})</span>
              </button>
              <button type="button" className="analysis-button" aria-label={`Stop watching ${watch.label}`} onClick={() => void remove(watch.id)}>
                Remove
              </button>
            </div>
            <p className="mt-1 text-muted">{lastCheckedAt ? <>Last checked <time dateTime={lastCheckedAt} className="tabular-nums">{formatDateTime(lastCheckedAt)}</time></> : 'Not checked yet'}</p>
            {lastDimensions.length > 0 && (
              <ul className="mt-1 space-y-0.5">
                {lastDimensions.map((result) => {
                  const { text, tone } = describeDimension(result);
                  return (
                    <li key={result.dimension} className={tone === 'missing' ? 'text-warning' : tone === 'changed' ? 'text-accent' : 'text-muted'}>
                      <span className="font-semibold">{DIMENSION_LABELS[result.dimension]}:</span> {text}
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
          <h4 className="analysis-label">{selected ? `Changes for ${selected.watch.label}` : 'Changes across all watches'}</h4>
          {changes.length === 0 ? (
            <p className="mt-1 text-muted">No changes recorded. This lists only changes that were detected; see each watch above for what a check could and could not observe.</p>
          ) : (
            <ul className="mt-1 space-y-2">
              {changes.map((change) => (
                <li key={change.id} className="rounded-lg border border-line p-2">
                  <div className="font-semibold text-fg">{CHANGE_LABELS[change.changeKind] ?? change.changeKind}</div>
                  <div className="text-muted">{change.subject}</div>
                  <dl className="mt-1 grid gap-0.5 sm:grid-cols-2">
                    <div>
                      <dt className="analysis-label">Before{change.oldAsOf ? `, as of ${change.oldAsOf}` : ''}</dt>
                      <dd>{change.oldValue ?? 'Not present at the baseline'}</dd>
                    </div>
                    <div>
                      <dt className="analysis-label">Now, as of {change.newAsOf}</dt>
                      <dd>{change.newValue ?? 'Not present now'}</dd>
                    </div>
                  </dl>
                  <p className="mt-1 text-muted">Basis: {change.basis}</p>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </section>
  );
}
