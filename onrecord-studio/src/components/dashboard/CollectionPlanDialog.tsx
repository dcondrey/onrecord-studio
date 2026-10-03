import { useCallback, useEffect, useMemo, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { CollectionPlanSource } from '../../types';
import DialogBackdrop from '../ui/DialogBackdrop';
import { guidanceForSubject } from '../../utils/sourceGuidance';

/** The subject most of the plan's sources were last collected for, to start the field from. */
export function lastSubject(sources: readonly CollectionPlanSource[]): string {
  const counts = new Map<string, number>();
  for (const { subject } of sources) {
    if (subject) counts.set(subject, (counts.get(subject) ?? 0) + 1);
  }
  let best = '';
  let bestCount = 0;
  for (const [subject, count] of counts) {
    if (count > bestCount) {
      best = subject;
      bestCount = count;
    }
  }
  return best;
}

/**
 * Opening the app collects nothing. This asks, once per launch, which saved sources to collect
 * from and for what subject; only "Start collecting" contacts any of them.
 */
export default function CollectionPlanDialog() {
  const [sources, setSources] = useState<CollectionPlanSource[] | null>(null);
  const [open, setOpen] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [subject, setSubject] = useState('');
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [newUrl, setNewUrl] = useState('');
  const [adding, setAdding] = useState(false);
  const [added, setAdded] = useState<string[]>([]);
  const [addError, setAddError] = useState<string | null>(null);
  const planGate = useLatestRequestGate();
  const startGate = useLatestRequestGate();
  const addGate = useLatestRequestGate();

  useEffect(() => {
    if (!isTauri()) return;
    const token = planGate.begin();
    invokeCommand('get_collection_plan', {})
      .then((plan) => {
        if (!planGate.isCurrent(token) || plan.sources.length === 0) return;
        setSources(plan.sources);
        setSelected(new Set(plan.sources.filter((source) => source.hasUnfinishedWork).map((source) => source.portalKey)));
        setSubject(lastSubject(plan.sources));
        setOpen(true);
      })
      .catch(() => {
        // Without a plan there is nothing to offer; saved sources stay available in the sidebar.
      });
    return planGate.invalidate;
  }, [planGate]);

  const close = useCallback(() => {
    startGate.invalidate();
    addGate.invalidate();
    setOpen(false);
  }, [addGate, startGate]);

  const toggle = (portalKey: string) =>
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(portalKey)) next.delete(portalKey);
      else next.add(portalKey);
      return next;
    });

  const start = () => {
    const token = startGate.begin();
    setPending(true);
    setError(null);
    invokeCommand('start_collection', { sourceKeys: [...selected], subject: subject.trim() || null })
      .then(() => {
        if (!startGate.isCurrent(token)) return;
        setPending(false);
        setOpen(false);
      })
      .catch((reason) => {
        if (!startGate.isCurrent(token)) return;
        setPending(false);
        setError(formatNativeError(reason));
      });
  };

  // A source added here starts at once, scoped to the subject typed above.
  const addSource = () => {
    const url = newUrl.trim();
    if (!url) return;
    const token = addGate.begin();
    const trimmedSubject = subject.trim();
    setAdding(true);
    setAddError(null);
    invokeCommand('start_data_source', {
      url,
      guidance: trimmedSubject ? guidanceForSubject(trimmedSubject) : null,
    })
      .then(({ source }) => {
        if (!addGate.isCurrent(token)) return;
        setAdding(false);
        setNewUrl('');
        setAdded((current) => [...current, source.displayName]);
      })
      .catch((reason) => {
        if (!addGate.isCurrent(token)) return;
        setAdding(false);
        setAddError(formatNativeError(reason));
      });
  };

  const unfinishedCount = useMemo(() => sources?.filter((source) => source.hasUnfinishedWork).length ?? 0, [sources]);

  if (!open || !sources) return null;
  return (
    <DialogBackdrop labelledBy="collection-plan-title" onClose={close} className="app-backdrop fixed inset-0 z-[100] grid place-items-center p-4 backdrop-blur-sm">
      <div className="app-panel flex max-h-[85vh] w-full max-w-lg flex-col overflow-hidden shadow-2xl shadow-black/40">
        <div className="border-b border-line p-4">
          <div className="app-eyebrow">Collection</div>
          <h2 id="collection-plan-title" className="mt-1 text-sm font-semibold text-fg">What should this session collect?</h2>
          <p className="mt-1 text-label leading-relaxed text-muted">
            Nothing is downloaded until you start.
            {unfinishedCount > 0 && ` ${unfinishedCount} source${unfinishedCount === 1 ? ' has' : 's have'} unfinished work from last time and ${unfinishedCount === 1 ? 'is' : 'are'} preselected.`}
          </p>
        </div>
        <div className="min-h-0 flex-1 space-y-4 overflow-y-auto p-4">
          <label className="block text-label font-medium text-fg-secondary">
            Subject
            <input
              type="text"
              value={subject}
              maxLength={200}
              onChange={(event) => setSubject(event.target.value)}
              placeholder="e.g. encampment sweeps, police overtime"
              className="focus-ring mt-1.5 h-10 w-full rounded-xl border border-control bg-canvas px-3 text-label text-fg"
            />
            <span className="mt-1 block font-normal text-muted">
              Records portals are searched for it; websites are walked, but only files whose link, name or page title mention every word are downloaded. Leave empty to collect everything.
            </span>
          </label>
          <fieldset>
            <legend className="text-label font-medium text-fg-secondary">Sources</legend>
            <ul className="mt-2 space-y-1">
              {sources.map((source) => (
                <li key={source.portalKey}>
                  <label className="flex cursor-pointer items-start gap-2 rounded-lg px-2 py-1.5 hover:bg-raised/60">
                    <input type="checkbox" className="mt-0.5" checked={selected.has(source.portalKey)} onChange={() => toggle(source.portalKey)} />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-label font-medium text-fg">{source.displayName}</span>
                      <span className="block truncate text-caption text-muted">
                        {source.sourceKind === 'NEXT_REQUEST' ? 'Records request portal' : 'Website'} · {source.canonicalUrl}
                        {source.hasUnfinishedWork && ' · unfinished'}
                      </span>
                    </span>
                  </label>
                </li>
              ))}
            </ul>
            <div className="mt-3">
              <label className="block text-label font-medium text-fg-secondary">
                Add a source
                <div className="mt-1.5 flex gap-2">
                  <input
                    type="url"
                    value={newUrl}
                    onChange={(event) => setNewUrl(event.target.value)}
                    placeholder="https://city.nextrequest.com or a public records page"
                    className="focus-ring h-10 min-w-0 flex-1 rounded-xl border border-control bg-canvas px-3 text-label text-fg"
                  />
                  <button type="button" onClick={addSource} disabled={adding || !newUrl.trim()} className="secondary-action focus-ring shrink-0 rounded-lg border px-3 py-2 text-label leading-4 text-fg-secondary disabled:opacity-50">
                    {adding ? 'Adding…' : 'Add and collect'}
                  </button>
                </div>
              </label>
              {added.length > 0 && <p role="status" className="mt-1 text-label text-success">Collecting from {added.join(', ')}.</p>}
              {addError && <p role="alert" className="mt-1 text-label leading-4 text-danger">{addError}</p>}
            </div>
          </fieldset>
          {error && <p role="alert" className="text-label leading-4 text-danger">{error}</p>}
        </div>
        <div className="flex justify-end gap-2 border-t border-line p-4">
          <button type="button" onClick={close} className="secondary-action focus-ring rounded-lg border px-3 py-2 text-label leading-4 text-fg-secondary">{added.length > 0 ? "Done" : "Not now"}</button>
          <button
            type="button"
            onClick={start}
            disabled={pending || selected.size === 0}
            className="primary-action focus-ring rounded-lg px-3 py-2 text-label leading-4 font-semibold text-white disabled:opacity-50"
          >
            {pending ? 'Starting…' : `Start collecting from ${selected.size} source${selected.size === 1 ? '' : 's'}`}
          </button>
        </div>
      </div>
    </DialogBackdrop>
  );
}
