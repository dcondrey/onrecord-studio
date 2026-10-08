import { useCallback, useEffect, useMemo, useState } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { CollectionPlanSource } from '../../types';
import Badge from '../ui/Badge';
import Button from '../ui/Button';
import Dialog, { DialogBody, DialogFooter, DialogHeader } from '../ui/Dialog';
import { Field, Input } from '../ui/Field';
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
    <Dialog onClose={close} size="md">
      <DialogHeader
        title="What should this session collect?"
        description={<>
          Nothing is downloaded until you start.
          {unfinishedCount > 0 && ` ${unfinishedCount} source${unfinishedCount === 1 ? ' has' : 's have'} unfinished work from last time and ${unfinishedCount === 1 ? 'is' : 'are'} preselected.`}
        </>}
      />
      <DialogBody className="space-y-5">
        <Field label="Subject" help="Separate searches with commas. Leave empty to collect everything.">
          <Input
            type="text"
            value={subject}
            maxLength={200}
            onChange={(event) => setSubject(event.target.value)}
            placeholder="e.g. encampment, safe parking"
          />
        </Field>
        <fieldset>
          <legend className="text-control text-fg-secondary">Sources</legend>
          <ul className="mt-2 space-y-1">
            {sources.map((source) => (
              <li key={source.portalKey}>
                <label className="flex cursor-pointer items-start gap-2.5 rounded-lg px-2 py-1.5 hover:bg-raised/60">
                  <input type="checkbox" className="mt-0.5 accent-[rgb(var(--accent))]" checked={selected.has(source.portalKey)} onChange={() => toggle(source.portalKey)} />
                  <span className="min-w-0 flex-1">
                    <span className="flex items-center gap-2">
                      <span className="truncate text-control text-fg">{source.displayName}</span>
                      {source.hasUnfinishedWork && <Badge tone="warning" className="shrink-0">Unfinished</Badge>}
                    </span>
                    <span className="block truncate text-caption text-muted" title={source.canonicalUrl}>
                      {source.sourceKind === 'NEXT_REQUEST' ? 'Records request portal' : 'Website'} · {source.canonicalUrl}
                    </span>
                  </span>
                </label>
              </li>
            ))}
          </ul>
        </fieldset>
        <div>
          <div className="flex items-end gap-2">
            <Field label="Add a source" className="min-w-0 flex-1">
              <Input
                type="url"
                value={newUrl}
                onChange={(event) => setNewUrl(event.target.value)}
                placeholder="Portal or page URL"
              />
            </Field>
            <Button onClick={addSource} disabled={adding || !newUrl.trim()} className="shrink-0">
              {adding ? 'Adding…' : 'Add and collect'}
            </Button>
          </div>
          {added.length > 0 && <p role="status" className="mt-1.5 text-caption text-success">Collecting from {added.join(', ')}.</p>}
          {addError && <p role="alert" className="mt-1.5 text-caption text-danger">{addError}</p>}
        </div>
        {error && <p role="alert" className="text-body text-danger">{error}</p>}
      </DialogBody>
      <DialogFooter>
        <Button variant="ghost" onClick={close}>{added.length > 0 ? 'Done' : 'Not now'}</Button>
        <Button variant="primary" onClick={start} disabled={pending || selected.size === 0}>
          {pending ? 'Starting…' : `Start collecting from ${selected.size} source${selected.size === 1 ? '' : 's'}`}
        </Button>
      </DialogFooter>
    </Dialog>
  );
}
