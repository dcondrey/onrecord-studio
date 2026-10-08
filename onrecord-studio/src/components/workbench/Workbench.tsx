import { useEffect, useRef, type KeyboardEvent, type ReactNode } from 'react';
import { useAppStore, type WorkbenchQueueLead, type WorkbenchSurface } from '../../store/useAppStore';
import { cn } from '../../utils/cn';

/**
 * The investigation workbench: a navigator (sources, watch status and the lead queue) on the left,
 * the selected lead in the centre, and the records around it on the right. `children` are the stage
 * pane and, unless `wide`, the context pane. Each pane scrolls on its own once there is room.
 */
export default function Workbench({ navigator, children, wide = false, className }: {
  navigator: ReactNode;
  children: ReactNode;
  /** No context pane: the stage takes its width too. */
  wide?: boolean;
  className?: string;
}) {
  return (
    <div className={cn('workbench min-h-0 flex-1', className)}>
      <div className="workbench-grid" data-wide={wide ? '' : undefined}>
        {navigator}
        {/* Below 960px the stage and context share this one scrolling column; a tab stop lets the
            keyboard scroll it while it holds only placeholders. */}
        <div className="workbench-main focus-ring" role="region" aria-label="Selected lead and the records around it" tabIndex={0}>{children}</div>
      </div>
    </div>
  );
}

/** One workbench pane: a titled, rounded module whose header stays put while its body scrolls. */
export function Pane({ label, title, meta, actions, children, className, bodyClassName, elevated = false }: {
  /** The pane's accessible name. */
  label: string;
  title?: ReactNode;
  meta?: ReactNode;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
  bodyClassName?: string;
  elevated?: boolean;
}) {
  return (
    <section aria-label={label} className={cn('workbench-pane flex flex-col rounded-xl border border-line bg-panel', elevated ? 'shadow-panel' : '', className)}>
      {(title || actions) && (
        <header className="sticky top-0 z-[1] flex h-11 shrink-0 items-center gap-2 rounded-t-xl border-b border-line bg-panel px-4">
          {title && <h2 className="text-overline uppercase text-muted">{title}</h2>}
          {meta && <span className="text-caption tabular-nums text-muted">{meta}</span>}
          {actions && <div className="ml-auto flex items-center gap-1">{actions}</div>}
        </header>
      )}
      <div className={cn('min-h-0 flex-1', bodyClassName)}>{children}</div>
    </section>
  );
}

/** A labelled group inside a pane: an overline heading, an optional count, then its rows. */
export function PaneSection({ title, count, action, children, className }: { title: string; count?: ReactNode; action?: ReactNode; children: ReactNode; className?: string }) {
  return (
    <section className={cn('px-4 py-3', className)}>
      <div className="mb-2 flex h-5 items-center gap-2">
        <h3 className="text-overline uppercase text-muted">{title}</h3>
        {count !== undefined && <span className="text-caption tabular-nums text-muted">{count}</span>}
        {action && <span className="ml-auto">{action}</span>}
      </div>
      {children}
    </section>
  );
}

const editable = (target: EventTarget | null) => target instanceof HTMLElement
  && (target.isContentEditable || ['INPUT', 'TEXTAREA', 'SELECT'].includes(target.tagName));

/**
 * Publishes the queue for the command bar and binds J and K to the next and previous lead, while
 * the workbench is mounted. Typing in a field, a modifier, or an open dialog leaves the keys alone.
 */
export function useWorkbenchQueue(surface: WorkbenchSurface, leads: readonly WorkbenchQueueLead[] | null, current: string | null) {
  const setWorkbenchQueue = useAppStore((state) => state.setWorkbenchQueue);
  const signature = leads ? leads.map((lead) => lead.id).join('|') : null;
  const latest = useRef(leads);
  latest.current = leads;
  useEffect(() => {
    setWorkbenchQueue(latest.current ? { surface, leads: [...latest.current], current } : null);
  }, [current, setWorkbenchQueue, signature, surface]);
  useEffect(() => () => setWorkbenchQueue(null), [setWorkbenchQueue]);
  useEffect(() => {
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.metaKey || event.ctrlKey || event.altKey || event.isComposing || editable(event.target)) return;
      if (document.querySelector('[aria-modal="true"]')) return;
      const key = event.key.toLowerCase();
      if (key !== 'j' && key !== 'k') return;
      event.preventDefault();
      useAppStore.getState().stepWorkbenchLead(key === 'j' ? 1 : -1);
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  }, []);
}

/**
 * The queue as a list of buttons with one tab stop: the arrow keys, Home and End move the
 * selection, and the selected lead is the current one (aria-current), so a screen reader hears
 * which lead the stage shows.
 */
export function LeadQueue({ label, items, selectedId, onSelect }: {
  label: string;
  items: readonly { id: string; content: ReactNode }[];
  selectedId: string | null;
  onSelect(id: string): void;
}) {
  const refs = useRef(new Map<string, HTMLButtonElement>());
  const index = items.findIndex((item) => item.id === selectedId);
  const tabStop = index < 0 ? items[0]?.id : selectedId;

  useEffect(() => {
    if (selectedId) refs.current.get(selectedId)?.scrollIntoView({ block: 'nearest' });
  }, [selectedId]);

  const onKeyDown = (event: KeyboardEvent<HTMLButtonElement>, at: number) => {
    const last = items.length - 1;
    const next = event.key === 'ArrowDown' ? Math.min(last, at + 1)
      : event.key === 'ArrowUp' ? Math.max(0, at - 1)
        : event.key === 'Home' ? 0
          : event.key === 'End' ? last
            : null;
    if (next === null) return;
    event.preventDefault();
    onSelect(items[next].id);
    refs.current.get(items[next].id)?.focus();
  };

  return (
    <ol aria-label={label} className="space-y-1 p-2">
      {items.map((item, at) => (
        <li key={item.id}>
          <button
            ref={(element) => { if (element) refs.current.set(item.id, element); else refs.current.delete(item.id); }}
            type="button"
            aria-current={item.id === selectedId ? 'true' : undefined}
            tabIndex={item.id === tabStop ? 0 : -1}
            onClick={() => onSelect(item.id)}
            onKeyDown={(event) => onKeyDown(event, at)}
            className="lead-card focus-ring block w-full rounded-lg border border-transparent px-3 py-2.5 text-left hover:bg-raised/60"
          >
            {item.content}
          </button>
        </li>
      ))}
    </ol>
  );
}
