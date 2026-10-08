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
import { absent } from '../charts/core/nulls';
import StateGrid, { type StateGridCell, type StateGridRow } from '../charts/figures/StateGrid';
import SegmentedControl from '../ui/SegmentedControl';

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

const DIMENSIONS = Object.keys(DIMENSION_LABELS) as WatchDimension[];

/** One cell of the watch outcome matrix: 'not observed' and 'unchanged' never share a glyph or words. */
function outcomeCell(label: string, dimension: WatchDimension, result: DimensionResult | undefined): StateGridCell {
  const where = `${label}, ${DIMENSION_LABELS[dimension].toLowerCase()}`;
  if (!result) {
    return { kind: 'gap', absence: absent('not-covered', 'not applicable to this watch kind'), glyph: 'bar', text: 'n/a', name: `${where}: not applicable to this watch kind.` };
  }
  const { text } = describeDimension(result);
  const span = result.baselineAsOf || result.observedAsOf
    ? [{ key: 'span', value: `${result.baselineAsOf ? formatShortDate(result.baselineAsOf) : 'no baseline'} → ${result.observedAsOf ? formatShortDate(result.observedAsOf) : 'not observed'}`, label: 'baseline → observed' }]
    : [];
  switch (result.outcome) {
    case 'CHANGED':
      return {
        kind: 'state', tone: 'lead', glyph: 'diamond', actionable: true,
        text: `${result.changeCount} changed`, tableText: text, name: `${where}: changed, ${text}. Press Enter to list these changes.`,
        tip: { rows: [{ key: 'n', value: result.changeCount.toLocaleString('en-US'), label: result.changeCount === 1 ? 'change' : 'changes', swatch: 'rect', tone: 'lead' }, ...span] },
      };
    case 'UNCHANGED':
      return { kind: 'state', tone: 'positive', glyph: 'check', text: 'same', tableText: text, name: `${where}: ${text}.`, tip: { rows: span, note: 'Observed and equal to the baseline.' } };
    case 'BASELINE_RECORDED':
      return { kind: 'state', tone: 'neutral', glyph: 'triangle', text: 'baseline', tableText: text, name: `${where}: ${text}.`, tip: { rows: span, note: 'First observation; nothing to compare yet.' } };
    case 'NOT_MEASURED':
      return {
        kind: 'gap', absence: absent('not-recorded', result.reason ?? 'the dimension could not be observed'), glyph: 'ring', text: 'not observed',
        tableText: `not observed (${result.reason ?? 'the dimension could not be observed'})`, name: `${where}: not observed. ${text}.`,
        tip: { rows: span, note: `Not observed: ${result.reason ?? 'the dimension could not be observed'}. This is not “unchanged”.` },
      };
  }
}

/** The matrix headline: changed watches and change total, then checks that could not observe. */
export function watchHeadline(summaries: readonly WatchSummary[]): string {
  const changed = summaries.filter((summary) => summary.lastDimensions.some((result) => result.outcome === 'CHANGED'));
  const total = summaries.reduce((sum, summary) => sum + summary.lastDimensions.reduce((inner, result) => inner + (result.outcome === 'CHANGED' ? result.changeCount : 0), 0), 0);
  const unobserved = summaries.reduce((sum, summary) => sum + summary.lastDimensions.filter((result) => result.outcome === 'NOT_MEASURED').length, 0);
  const lead = `${changed.length} of ${summaries.length} ${summaries.length === 1 ? 'watch' : 'watches'} changed since ${summaries.length === 1 ? 'its' : 'their'} last check (${total.toLocaleString('en-US')} ${total === 1 ? 'change' : 'changes'})`;
  if (unobserved === 0) return `${lead}.`;
  return `${lead}; ${unobserved} dimension ${unobserved === 1 ? 'check' : 'checks'} could not observe ${unobserved === 1 ? 'its' : 'their'} source.`;
}

function relativeAge(iso: string | null): string {
  if (!iso) return 'not checked yet';
  const ms = Date.now() - Date.parse(iso);
  if (!Number.isFinite(ms)) return 'not checked yet';
  const minutes = Math.max(0, Math.round(ms / 60_000));
  if (minutes < 60) return `${minutes} min ago`;
  const hours = Math.round(minutes / 60);
  if (hours < 48) return `${hours} h ago`;
  return `${Math.round(hours / 24)} d ago`;
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
  const [dimensionFilter, setDimensionFilter] = useState<WatchDimension | null>(null);
  const [view, setView] = useState<'matrix' | 'list'>('matrix');
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
  const shownChanges = dimensionFilter ? changes.filter((change) => change.dimension === dimensionFilter) : changes;
  const selectWatch = (id: string | null) => {
    setSelectedId(id);
    setDimensionFilter(null);
  };
  const gridRows: StateGridRow[] = summaries.map(({ watch, lastCheckedAt, lastDimensions }) => ({
    key: watch.id,
    label: watch.label,
    sublabel: KIND_LABELS[watch.kind],
    trailing: { text: relativeAge(lastCheckedAt) },
    cells: DIMENSIONS.map((dimension) => outcomeCell(watch.label, dimension, lastDimensions.find((result) => result.dimension === dimension))),
  }));
  const selectedRow = summaries.findIndex((summary) => summary.watch.id === selectedId);

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
      {summaries.length > 0 && (
        <SegmentedControl
          label="Watch outcomes as"
          value={view}
          mode="filter"
          size="sm"
          onChange={setView}
          options={[{ value: 'matrix', label: 'Matrix' }, { value: 'list', label: 'List view' }]}
        />
      )}
      {view === 'matrix' && summaries.length > 0 && (
        <StateGrid
          title="Watch outcomes"
          storageKey="watch-outcomes"
          headline={watchHeadline(summaries)}
          sources={{ tables: ['watches', 'watch_runs', 'watch_baselines', 'watch_changes'], note: 'not observed ≠ unchanged' }}
          summary={`${summaries.length} watches by ${DIMENSIONS.length} dimensions. Changed cells can be activated to list their changes below.`}
          status="ready"
          rowHeader="Watch"
          trailingHeader="Checked"
          columns={DIMENSIONS.map((dimension) => ({ key: dimension, label: DIMENSION_LABELS[dimension] }))}
          rows={gridRows}
          selected={selectedRow >= 0 && dimensionFilter ? [{ row: selectedRow, col: DIMENSIONS.indexOf(dimensionFilter) }] : []}
          onActivate={(row, col) => {
            setSelectedId(summaries[row].watch.id);
            setDimensionFilter(DIMENSIONS[col]);
          }}
          onEscape={() => selectWatch(null)}
          legend={[
            { key: 'changed', label: 'Changed (a lead to read)', swatch: 'rect', tone: 'lead' },
            { key: 'same', label: 'Observed, unchanged', swatch: 'dot', tone: 'positive' },
            { key: 'baseline', label: 'Baseline recorded', swatch: 'hollow', tone: 'mark' },
          ]}
        />
      )}
      <ul className="space-y-2">
        {summaries.map(({ watch, lastCheckedAt, lastDimensions }) => (
          <li key={watch.id} className={cn('rounded-lg border p-3', watch.id === selectedId ? 'border-accent/30 bg-accent/5' : 'border-line')}>
            <div className="flex flex-wrap items-center justify-between gap-2">
              <button type="button" className="focus-ring rounded text-left font-medium text-fg hover:underline" aria-pressed={watch.id === selectedId} onClick={() => selectWatch(watch.id === selectedId ? null : watch.id)}>
                {watch.label} <span className="font-normal text-muted">· {KIND_LABELS[watch.kind]}</span>
              </button>
              <Button size="sm" variant="ghost" aria-label={`Stop watching ${watch.label}`} onClick={() => void remove(watch.id)}>
                Remove
              </Button>
            </div>
            <p className="mt-1 text-caption text-muted">{lastCheckedAt ? <>Last checked <Time value={lastCheckedAt} format={formatDateTime} className="whitespace-nowrap" /></> : 'Not checked yet'}</p>
            {view === 'list' && lastDimensions.length > 0 && (
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
          <div className="flex flex-wrap items-center gap-2">
            <h4 className="text-body font-medium text-fg">
              {selected ? `Changes for ${selected.watch.label}` : 'Changes across all watches'}
              {dimensionFilter ? ` · ${DIMENSION_LABELS[dimensionFilter]}` : ''}
            </h4>
            {dimensionFilter && <Button size="sm" variant="ghost" onClick={() => setDimensionFilter(null)}>Show every dimension</Button>}
          </div>
          {shownChanges.length === 0 ? (
            <p className="mt-1 text-muted">No changes recorded. This lists only changes that were detected; see each watch above for what a check could and could not observe.</p>
          ) : (
            <ul className="mt-1 space-y-2">
              {shownChanges.map((change) => (
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
