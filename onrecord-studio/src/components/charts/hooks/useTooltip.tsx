import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { createPortal } from 'react-dom';
import { TooltipLayer, type TooltipAnchor } from '../frame/Tooltip';

interface Shown {
  content: ReactNode;
  at: TooltipAnchor;
}

/**
 * One tooltip per frame, rendered in a portal. Pointer positions are applied once per animation
 * frame; a DOMRect (a focused mark's box) is applied at once. Content is React nodes, so labels
 * from CSV or IPC render as text. Every value in a tooltip must also be in a direct label or the
 * table twin: a tooltip never gates information.
 */
export function useTooltip() {
  const [shown, setShown] = useState<Shown | null>(null);
  const pending = useRef<Shown | null>(null);
  const frame = useRef(0);

  const cancel = () => {
    if (frame.current) cancelAnimationFrame(frame.current);
    frame.current = 0;
  };

  const show = useCallback((content: ReactNode, at: TooltipAnchor) => {
    if ('width' in at || typeof requestAnimationFrame === 'undefined') {
      cancel();
      setShown({ content, at });
      return;
    }
    pending.current = { content, at: { clientX: at.clientX, clientY: at.clientY } };
    if (!frame.current) {
      frame.current = requestAnimationFrame(() => {
        frame.current = 0;
        setShown(pending.current);
      });
    }
  }, []);

  const hide = useCallback(() => {
    cancel();
    pending.current = null;
    setShown(null);
  }, []);

  useEffect(() => cancel, []);

  // A pointer tooltip would detach from the pointer when anything scrolls, so it closes instead. A
  // focus tooltip stays: moving focus can itself scroll the mark into view, and blur closes it.
  useEffect(() => {
    if (!shown || 'width' in shown.at) return;
    window.addEventListener('scroll', hide, { capture: true, passive: true });
    return () => window.removeEventListener('scroll', hide, { capture: true });
  }, [shown, hide]);

  const node = shown && typeof document !== 'undefined' ? createPortal(<TooltipLayer anchor={shown.at}>{shown.content}</TooltipLayer>, document.body) : null;
  return useMemo(() => ({ show, hide, node, visible: shown !== null }), [show, hide, node, shown]);
}
