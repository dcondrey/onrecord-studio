import { createContext, useContext, useEffect, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import { useAppStore, type InspectorTarget } from '../../store/useAppStore';
import { cn } from '../../utils/cn';
import Icon, { type IconName } from '../ui/icons';
import { cachedPeek, loadPeek, type PeekData } from './peek';

const PEEK_DELAY_MS = 450;
const PEEK_CLOSE_GRACE_MS = 140;
const PEEK_WIDTH = 320;

/** True inside an inspector layer, where opening an object stacks a layer instead of replacing the stack. */
export const InspectorLayerContext = createContext(false);

export const OBJECT_ICONS: Record<InspectorTarget['kind'], IconName> = {
  finding: 'flag',
  entity: 'tag',
  document: 'evidence',
  agency: 'building',
  request: 'clipboard',
  case: 'briefcase',
};

let closeOpenPeek: (() => void) | null = null;

export type PeekState = { status: 'pending' } | { status: 'ready'; data: PeekData } | { status: 'failed' };

/**
 * A reference to a record, finding, entity, agency or request. Hovering previews it; clicking
 * opens it in the inspector without leaving the current view.
 */
export default function ObjectLink({ target, children, className }: { target: InspectorTarget; children: ReactNode; className?: string }) {
  const inspect = useAppStore((state) => state.inspect);
  const withinLayer = useContext(InspectorLayerContext);
  const anchor = useRef<HTMLButtonElement>(null);
  const openTimer = useRef<number | undefined>(undefined);
  const closeTimer = useRef<number | undefined>(undefined);
  const [position, setPosition] = useState<{ left: number; top: number; above: boolean } | null>(null);
  const [peek, setPeek] = useState<PeekState>({ status: 'pending' });
  const gate = useLatestRequestGate();

  const close = () => {
    window.clearTimeout(openTimer.current);
    window.clearTimeout(closeTimer.current);
    gate.invalidate();
    setPosition(null);
    if (closeOpenPeek === close) closeOpenPeek = null;
  };

  const show = () => {
    const rect = anchor.current?.getBoundingClientRect();
    if (!rect) return;
    if (closeOpenPeek && closeOpenPeek !== close) closeOpenPeek();
    closeOpenPeek = close;
    const above = rect.bottom + 220 > window.innerHeight;
    setPosition({
      left: Math.max(8, Math.min(rect.left, window.innerWidth - PEEK_WIDTH - 8)),
      top: above ? rect.top - 8 : rect.bottom + 8,
      above,
    });
    const cached = cachedPeek(target);
    if (cached) {
      setPeek({ status: 'ready', data: cached });
      return;
    }
    const token = gate.begin();
    setPeek({ status: 'pending' });
    loadPeek(target)
      .then((data) => { if (gate.isCurrent(token)) setPeek({ status: 'ready', data }); })
      .catch(() => { if (gate.isCurrent(token)) setPeek({ status: 'failed' }); });
  };

  const scheduleOpen = () => {
    window.clearTimeout(closeTimer.current);
    window.clearTimeout(openTimer.current);
    openTimer.current = window.setTimeout(show, PEEK_DELAY_MS);
  };

  const scheduleClose = () => {
    window.clearTimeout(openTimer.current);
    closeTimer.current = window.setTimeout(close, PEEK_CLOSE_GRACE_MS);
  };

  // The peek describes this target only; a link re-pointed at another target drops it.
  const targetKey = JSON.stringify(target);
  useEffect(() => close, [targetKey]);

  return (
    <>
      <button
        ref={anchor}
        type="button"
        onMouseEnter={scheduleOpen}
        onMouseLeave={scheduleClose}
        onClick={() => {
          close();
          inspect(target, withinLayer ? 'layer' : 'view');
        }}
        className={cn('focus-ring rounded-sm text-left text-accent underline decoration-accent/30 underline-offset-2 transition hover:decoration-accent', className)}
      >
        {children}
      </button>
      {position && createPortal(
        <div
          role="tooltip"
          onMouseEnter={() => window.clearTimeout(closeTimer.current)}
          onMouseLeave={scheduleClose}
          style={{ left: position.left, top: position.top, width: PEEK_WIDTH, transform: position.above ? 'translateY(-100%)' : undefined }}
          className="fixed z-[110]"
        >
          <div className="motion-pop rounded-xl border border-line-strong bg-canvas p-3 text-label shadow-2xl">
            <PeekBody kind={target.kind} state={peek} />
          </div>
        </div>,
        document.body,
      )}
    </>
  );
}

export function PeekBody({ kind, state }: { kind: InspectorTarget['kind']; state: PeekState }) {
  if (state.status === 'pending') {
    return (
      <div className="space-y-2" aria-label="Loading preview">
        <div className="skeleton h-3 w-24 rounded" />
        <div className="skeleton h-4 w-full rounded" />
        <div className="skeleton h-3 w-2/3 rounded" />
      </div>
    );
  }
  if (state.status === 'failed') return <p className="text-muted">The preview could not be loaded. Click to open it.</p>;
  const { data } = state;
  return (
    <div>
      <div className="app-eyebrow flex items-center gap-1.5 text-accent"><Icon name={OBJECT_ICONS[kind]} className="h-3 w-3" /> {data.eyebrow}</div>
      <p className="mt-1 font-semibold leading-snug text-fg">{data.title}</p>
      {data.excerpt && (
        <p className="mt-2 line-clamp-5 rounded-md border border-line bg-canvas/50 p-2 text-caption leading-relaxed text-fg-secondary">
          {data.excerpt.before}
          {data.excerpt.marked && <mark className="rounded-sm bg-warning/20 px-0.5 text-fg">{data.excerpt.marked}</mark>}
          {data.excerpt.after}
        </p>
      )}
      <ul className="mt-2 space-y-0.5 text-caption text-muted">
        {data.lines.map((line) => <li key={line}>{line}</li>)}
      </ul>
      <p className="mt-2 text-caption text-muted">Click to open</p>
    </div>
  );
}
