import { useEffect, useRef } from 'react';
import { inspectorTargetKey, useAppStore, type InspectorTarget } from '../../store/useAppStore';
import { cn } from '../../utils/cn';
import Icon from '../ui/icons';
import { LayerBody } from './layers';
import { InspectorLayerContext, OBJECT_ICONS } from './ObjectLink';

const KIND_LABELS: Record<InspectorTarget['kind'], string> = {
  finding: 'Finding',
  entity: 'Entity',
  document: 'Document',
  agency: 'Agency',
  request: 'Request',
  case: 'Case',
};

export function targetTitle(target: InspectorTarget): string {
  switch (target.kind) {
    case 'finding': return target.label ?? 'Finding';
    case 'entity': return target.label;
    case 'document': return target.label ?? `Document ${target.documentId}`;
    case 'agency': return target.label;
    case 'request': return target.label ?? `Request ${target.requestId}`;
    case 'case': return target.label ?? 'Case';
  }
}

/**
 * The detail layer every view shares. Opening an object from a view starts a stack; following a
 * link inside a layer stacks another, and the trail at the top walks back through them. The view
 * underneath stays where it was.
 */
export default function Inspector() {
  const stack = useAppStore((state) => state.inspectorStack);
  const inspect = useAppStore((state) => state.inspect);
  const back = useAppStore((state) => state.inspectorBack);
  const close = useAppStore((state) => state.closeInspector);
  const heading = useRef<HTMLHeadingElement>(null);
  const opener = useRef<HTMLElement | null>(null);
  const top = stack.at(-1) ?? null;
  const topKey = top ? inspectorTargetKey(top) : null;
  const isOpen = top !== null;

  useEffect(() => {
    if (isOpen && !opener.current && document.activeElement instanceof HTMLElement) opener.current = document.activeElement;
    if (!isOpen && opener.current) {
      opener.current.focus();
      opener.current = null;
    }
  }, [isOpen]);

  useEffect(() => {
    if (topKey) heading.current?.focus();
  }, [topKey]);

  useEffect(() => {
    if (!isOpen) return;
    const onKeyDown = (event: KeyboardEvent) => {
      if (event.key !== 'Escape' || event.defaultPrevented) return;
      // A modal dialog above the inspector owns Escape.
      if (document.querySelector('[aria-modal="true"]')) return;
      event.preventDefault();
      back();
    };
    window.addEventListener('keydown', onKeyDown);
    return () => window.removeEventListener('keydown', onKeyDown);
  }, [back, isOpen]);

  if (!top) return null;

  return (
    <aside
      aria-labelledby="inspector-title"
      className="motion-slide-in fixed inset-y-0 right-0 z-[90] flex w-full max-w-[480px] flex-col border-l border-line-strong bg-canvas shadow-2xl"
    >
      <header className="shrink-0 border-b border-line px-4 pb-3 pt-3">
        <div className="flex items-center gap-2">
          <button type="button" onClick={back} aria-label={stack.length > 1 ? `Back to ${targetTitle(stack[stack.length - 2])}` : 'Close inspector'} className="motion-press focus-ring grid h-7 w-7 place-items-center rounded-lg border border-control text-muted hover:text-fg">
            <Icon name={stack.length > 1 ? 'back' : 'x'} className="h-3.5 w-3.5" />
          </button>
          {stack.length > 1 && (
            <nav aria-label="Inspector trail" className="flex min-w-0 flex-1 items-center gap-1 overflow-hidden text-caption text-muted">
              {stack.slice(0, -1).map((entry, index) => (
                <span key={inspectorTargetKey(entry)} className="flex min-w-0 items-center gap-1">
                  {index > 0 && <Icon name="chevron" className="h-3 w-3 shrink-0" />}
                  <button type="button" onClick={() => inspect(entry, 'layer')} className="focus-ring truncate rounded hover:text-fg">{targetTitle(entry)}</button>
                </span>
              ))}
            </nav>
          )}
          <button type="button" onClick={close} aria-label="Close inspector" className={cn('motion-press focus-ring ml-auto grid h-7 w-7 place-items-center rounded-lg text-muted hover:text-fg', stack.length === 1 && 'hidden')}>
            <Icon name="x" className="h-3.5 w-3.5" />
          </button>
        </div>
        <div className="app-eyebrow mt-3 flex items-center gap-1.5 text-accent">
          <Icon name={OBJECT_ICONS[top.kind]} className="h-3.5 w-3.5" /> {KIND_LABELS[top.kind]}
        </div>
        <h2 ref={heading} id="inspector-title" tabIndex={-1} className="mt-1 text-[17px] font-semibold leading-snug tracking-[-0.01em] text-fg outline-none">
          {targetTitle(top)}
        </h2>
      </header>
      <div key={topKey} className="motion-reveal min-h-0 flex-1 overflow-y-auto px-4 py-4">
        <InspectorLayerContext.Provider value>
          <LayerBody target={top} />
        </InspectorLayerContext.Provider>
      </div>
      <footer className="shrink-0 border-t border-line px-4 py-2 text-caption text-muted">
        Esc steps back · links open as layers
      </footer>
    </aside>
  );
}
