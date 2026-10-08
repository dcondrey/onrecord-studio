import { useEffect, useRef, useState, type KeyboardEvent } from 'react';
import { invokeCommand } from '../../ipc';
import { useGatedLoad } from '../../hooks/useGatedLoad';
import { useAppStore } from '../../store/useAppStore';
import type { CaseworkState } from '../../types';
import { cn } from '../../utils/cn';
import Button from '../ui/Button';
import Icon from '../ui/icons';
import Skeleton from '../ui/Skeleton';

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
  const menu = useRef<HTMLDivElement>(null);
  const trigger = useRef<HTMLButtonElement>(null);
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
    window.addEventListener('pointerdown', onPointer);
    return () => window.removeEventListener('pointerdown', onPointer);
  }, [open]);

  const label = active ? active.title : activeMissing ? 'Case not found' : 'No case';

  const items = () => Array.from(menu.current?.querySelectorAll<HTMLElement>('[role="menuitem"], [role="menuitemradio"]') ?? []);
  const close = (returnFocus: boolean) => {
    setOpen(false);
    if (returnFocus) trigger.current?.focus();
  };

  // Focus enters the menu on open: the case being worked, or the first item.
  useEffect(() => {
    if (!open) return;
    const all = items();
    (all.find((item) => item.getAttribute('aria-checked') === 'true') ?? all[0])?.focus();
  }, [open, casework.status, cases.length]);

  const onMenuKeyDown = (event: KeyboardEvent<HTMLDivElement>) => {
    const all = items();
    const index = all.indexOf(document.activeElement as HTMLElement);
    const move = (next: number) => { event.preventDefault(); all[(next + all.length) % all.length]?.focus(); };
    if (event.key === 'ArrowDown') move(index + 1);
    else if (event.key === 'ArrowUp') move(index < 0 ? all.length - 1 : index - 1);
    else if (event.key === 'Home') move(0);
    else if (event.key === 'End') move(all.length - 1);
    else if (event.key === 'Tab') { event.preventDefault(); close(true); }
    else if (event.key === 'Escape') { event.preventDefault(); close(true); }
  };

  return (
    <div ref={root} className="relative shrink-0">
      <button
        ref={trigger}
        type="button"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-label={`Case: ${label}`}
        onClick={() => { setOpen((current) => !current); setLoads((count) => count + 1); }}
        title={active ? `Working: ${active.title}` : 'Choose the case you are working'}
        className={cn(
          'motion-press focus-ring flex h-8 items-center gap-1.5 rounded-lg border text-control transition-colors',
          active || activeMissing ? 'px-2.5' : 'w-8 justify-center',
          activeMissing ? 'border-warning/30 bg-warning/10 text-warning'
            : active ? 'border-line-strong text-fg-secondary hover:bg-raised hover:text-fg'
              : 'border-transparent text-fg-secondary hover:bg-raised hover:text-fg',
        )}
      >
        <Icon name="briefcase" size="md" className="shrink-0" />
        {(active || activeMissing) && <span className="hidden max-w-[10rem] truncate xl:inline">{label}</span>}
      </button>
      {open && (
        <div
          ref={menu}
          role="menu"
          aria-label="Cases"
          onKeyDown={onMenuKeyDown}
          onBlur={(event) => {
            if (!root.current?.contains(event.relatedTarget as Node | null)) close(false);
          }}
          className="glass motion-pop absolute right-0 top-full z-popover mt-2 w-96 overflow-hidden rounded-xl p-1.5"
        >
          {(casework.status === 'pending' || (casework.status === 'ready' && casework.data === null)) && (
            <Skeleton variant="lines" lines={3} label="Loading cases" className="p-2" />
          )}
          {casework.status === 'failed' && <p role="alert" className="p-2 text-body text-danger">Cases could not be loaded: {casework.message}</p>}
          {casework.status === 'ready' && casework.data !== null && cases.length === 0 && (
            <div className="p-3">
              <p className="text-control text-fg-secondary">No cases yet</p>
              <p className="mt-1 text-caption text-muted">A case starts from a research report in casework.</p>
              <Button size="sm" role="menuitem" className="mt-3" onClick={() => { setOpen(false); openCasework(''); }}>Open casework</Button>
            </div>
          )}
          {cases.map((entry) => {
            const checked = entry.id === activeCaseId;
            return (
              <div key={entry.id} className="flex items-center gap-1 rounded-lg hover:bg-raised/60">
                <button
                  type="button"
                  role="menuitemradio"
                  aria-checked={checked}
                  tabIndex={-1}
                  onClick={() => { setActiveCase(entry.id); close(true); }}
                  className="flex min-w-0 flex-1 items-start gap-2 rounded-lg px-2 py-2 text-left outline-none focus-visible:bg-raised"
                >
                  <span className="grid h-5 w-4 shrink-0 place-items-center text-accent" aria-hidden="true">
                    {checked && (
                      <svg viewBox="0 0 16 16" className="h-3.5 w-3.5" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                        <path d="m3.5 8.5 3 3 6-7" />
                      </svg>
                    )}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-body font-medium text-fg">{entry.title}</span>
                    <span className="line-clamp-2 text-caption tabular-nums text-muted [text-wrap:pretty]">
                      {entry.topic || 'No topic'}
                      {/* No-break spaces keep each count whole, so a wrap never strands "· 1" from its noun. */}
                      {` ·\u00a0${entry.evidence.length}\u00a0source${entry.evidence.length === 1 ? '' : 's'} ·\u00a0${entry.requests.length}\u00a0request${entry.requests.length === 1 ? '' : 's'}`}
                    </span>
                  </span>
                </button>
                <button
                  type="button"
                  role="menuitem"
                  tabIndex={-1}
                  aria-label={`Open ${entry.title}`}
                  title={`Open ${entry.title}`}
                  onClick={() => { setOpen(false); inspect({ kind: 'case', caseId: entry.id, label: entry.title }, 'view'); }}
                  className="mr-1 grid h-7 w-7 shrink-0 place-items-center rounded-md text-muted outline-none transition-colors hover:bg-raised hover:text-fg focus-visible:bg-raised focus-visible:text-fg"
                >
                  <Icon name="chevron" size="sm" />
                </button>
              </div>
            );
          })}
          {activeCaseId !== null && (
            <button type="button" role="menuitem" tabIndex={-1} onClick={() => { setActiveCase(null); close(true); }} className="mt-1 w-full rounded-lg border-t border-line px-3 py-2 text-left text-caption text-muted outline-none hover:text-fg focus-visible:bg-raised focus-visible:text-fg">
              {activeMissing ? 'Clear the missing case' : 'Stop working this case'}
            </button>
          )}
        </div>
      )}
    </div>
  );
}
