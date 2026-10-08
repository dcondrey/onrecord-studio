import { useEffect, useRef, useState } from 'react';
import { invokeCommand } from '../../ipc';
import { useGatedLoad } from '../../hooks/useGatedLoad';
import { useAppStore } from '../../store/useAppStore';
import type { CaseworkState } from '../../types';
import { cn } from '../../utils/cn';
import Icon from '../ui/icons';

/**
 * The case being worked, one click from every view. Choosing a case keeps it; opening it shows its
 * requests and cited sources in the inspector.
 */
export default function CaseSwitcher() {
  const activeCaseId = useAppStore((state) => state.activeCaseId);
  const setActiveCase = useAppStore((state) => state.setActiveCase);
  const inspect = useAppStore((state) => state.inspect);
  const openCasework = useAppStore((state) => state.openCasework);
  const [open, setOpen] = useState(false);
  const [loads, setLoads] = useState(0);
  const root = useRef<HTMLDivElement>(null);
  // Nothing to show until a case is chosen or the menu opens, so no case means no load.
  const shouldLoad = activeCaseId !== null || loads > 0;
  const casework = useGatedLoad<CaseworkState | null>(`cases:${shouldLoad}:${loads}`, () => shouldLoad ? invokeCommand('get_casework_state', {}) : Promise.resolve(null));
  const cases = casework.status === 'ready' && casework.data ? casework.data.cases : [];
  const active = cases.find((entry) => entry.id === activeCaseId) ?? null;
  const activeMissing = casework.status === 'ready' && casework.data !== null && activeCaseId !== null && active === null;

  useEffect(() => {
    if (!open) return;
    const onPointer = (event: PointerEvent) => {
      if (!root.current?.contains(event.target as Node)) setOpen(false);
    };
    const onKey = (event: KeyboardEvent) => {
      if (event.key === 'Escape') { event.preventDefault(); setOpen(false); }
    };
    window.addEventListener('pointerdown', onPointer);
    window.addEventListener('keydown', onKey);
    return () => {
      window.removeEventListener('pointerdown', onPointer);
      window.removeEventListener('keydown', onKey);
    };
  }, [open]);

  const label = active ? active.title : activeMissing ? 'Case not found' : 'No case';

  return (
    <div ref={root} className="relative shrink-0">
      <button
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        onClick={() => { setOpen((current) => !current); setLoads((count) => count + 1); }}
        title={active ? `Working: ${active.title}` : 'Choose the case you are working'}
        className={cn(
          'motion-press focus-ring flex h-9 max-w-[14rem] items-center gap-1.5 rounded-xl border px-2.5 text-label font-medium transition',
          active ? 'border-highlight/30 bg-highlight/10 text-highlight' : activeMissing ? 'border-warning/30 bg-warning/10 text-warning' : 'border-control bg-surface/70 text-muted hover:text-fg',
        )}
      >
        <Icon name="briefcase" className="h-4 w-4 shrink-0" />
        <span className="hidden truncate 2xl:inline">{label}</span>
        <span className="sr-only 2xl:hidden">Case: {label}</span>
      </button>
      {open && (
        <div role="menu" aria-label="Cases" className="motion-pop absolute right-0 top-full z-[95] mt-2 w-80 overflow-hidden rounded-xl border border-line-strong bg-canvas p-1.5 shadow-2xl">
          {(casework.status === 'pending' || (casework.status === 'ready' && casework.data === null)) && <div role="status" className="space-y-1.5 p-2"><div className="skeleton h-4 w-2/3 rounded" /><div className="skeleton h-4 w-1/2 rounded" /></div>}
          {casework.status === 'failed' && <p role="alert" className="p-2 text-label text-danger">Cases could not be loaded: {casework.message}</p>}
          {casework.status === 'ready' && casework.data !== null && cases.length === 0 && (
            <div className="p-3 text-label">
              <p className="font-medium text-fg-secondary">No cases yet</p>
              <p className="mt-1 text-caption text-muted">A case starts from a research report in casework.</p>
              <button type="button" role="menuitem" onClick={() => { setOpen(false); openCasework(''); }} className="focus-ring mt-2 rounded-lg border border-control px-2.5 py-1 text-label font-semibold text-fg-secondary hover:text-fg">Open casework</button>
            </div>
          )}
          {cases.map((entry) => (
            <div key={entry.id} className={cn('group flex items-center gap-1 rounded-lg', entry.id === activeCaseId && 'bg-highlight/10')}>
              <button
                type="button"
                role="menuitemradio"
                aria-checked={entry.id === activeCaseId}
                onClick={() => { setActiveCase(entry.id); setOpen(false); }}
                className="focus-ring min-w-0 flex-1 rounded-lg px-2.5 py-2 text-left"
              >
                <span className="block truncate text-label font-medium text-fg">{entry.title}</span>
                <span className="block truncate text-caption text-muted">{entry.topic || 'No topic'} · {entry.evidence.length} source{entry.evidence.length === 1 ? '' : 's'} · {entry.requests.length} request{entry.requests.length === 1 ? '' : 's'}</span>
              </button>
              <button
                type="button"
                role="menuitem"
                aria-label={`Open ${entry.title}`}
                onClick={() => { setOpen(false); inspect({ kind: 'case', caseId: entry.id, label: entry.title }, 'view'); }}
                className="focus-ring mr-1 grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted opacity-0 transition hover:bg-raised hover:text-fg focus:opacity-100 group-hover:opacity-100"
              >
                <Icon name="chevron" className="h-3.5 w-3.5" />
              </button>
            </div>
          ))}
          {activeCaseId !== null && (
            <button type="button" role="menuitem" onClick={() => { setActiveCase(null); setOpen(false); }} className="focus-ring mt-1 w-full rounded-lg border-t border-line px-2.5 py-2 text-left text-caption text-muted hover:text-fg">
              {activeMissing ? 'Clear the missing case' : 'Stop working this case'}
            </button>
          )}
        </div>
      )}
    </div>
  );
}
