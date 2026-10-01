import { useMemo, useState } from 'react';
import type { CaseworkCase } from '../../types/casework';
import { applyMerge, mergeCandidates, type MergeChange } from '../../utils/caseworkMerge';

const CHANGE_LABELS: Record<MergeChange, string> = {
  added: 'Only in the shared case',
  changed: 'Different in the shared case',
  removed: 'Missing from the shared case',
};

/**
 * Item-level merge of a shared case into the case that is open. Nothing is applied until the
 * reader ticks it, nothing is saved by applying, and a removal is never pre-selected: a shared
 * case that lacks an item may simply be older, which says nothing about whether it was deleted.
 */
export default function CaseMerge({
  local,
  incoming,
  onApply,
}: {
  local: CaseworkCase;
  incoming: CaseworkCase;
  onApply: (merged: CaseworkCase, count: number) => void;
}) {
  const candidates = useMemo(() => mergeCandidates(local, incoming), [local, incoming]);
  const [selected, setSelected] = useState<ReadonlySet<string>>(new Set());
  if (candidates.length === 0) return <p className="text-xs text-slate-400">The open case and the shared case have the same items.</p>;
  const toggle = (key: string) =>
    setSelected((current) => {
      const next = new Set(current);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  return (
    <section aria-label="Merge selected items into the open case" className="rounded border border-slate-700 p-3">
      <h5 className="text-sm font-semibold text-slate-100">Merge selected items into your open case</h5>
      <p className="mt-1 text-xs text-slate-400">Tick what to take. The result is a draft in the open case; review it, then save a new revision. Nothing is saved by this step, and a shared case that lacks an item may only be older.</p>
      <div className="mt-2 flex gap-3 text-xs">
        <button type="button" className="underline" onClick={() => setSelected(new Set(candidates.filter((c) => c.change === 'added').map((c) => c.key)))}>Select additions</button>
        <button type="button" className="underline" onClick={() => setSelected(new Set())}>Clear</button>
      </div>
      <ul className="mt-2 space-y-1 text-xs text-slate-300">
        {candidates.map((candidate) => (
          <li key={candidate.key}>
            <label className="flex items-start gap-2">
              <input type="checkbox" checked={selected.has(candidate.key)} onChange={() => toggle(candidate.key)} />
              <span>
                {candidate.label} <span className={candidate.change === 'removed' ? 'text-amber-300' : 'text-slate-400'}>· {CHANGE_LABELS[candidate.change]}</span>
              </span>
            </label>
          </li>
        ))}
      </ul>
      <button type="button" className="analysis-button mt-3" disabled={selected.size === 0} onClick={() => onApply(applyMerge(local, incoming, selected), selected.size)}>
        Apply {selected.size || ''} selected to my draft
      </button>
    </section>
  );
}
