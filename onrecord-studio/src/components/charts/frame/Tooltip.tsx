import { useLayoutEffect, useRef, useState, type ReactNode } from 'react';
import { LegendSwatch, type SwatchKind } from './Legend';

export type TooltipAnchor = { clientX: number; clientY: number } | DOMRect;

const EDGE = 8;
const OFFSET = 12;

function isRect(anchor: TooltipAnchor): anchor is DOMRect {
  return 'width' in anchor;
}

/**
 * The one floating tooltip of a frame. It sits beside the pointer and flips at the viewport's edges;
 * anchored to a mark's box (keyboard focus) it sits above the mark, or below when there is no room.
 */
export function TooltipLayer({ anchor, children }: { anchor: TooltipAnchor; children: ReactNode }) {
  const box = useRef<HTMLDivElement>(null);
  const [position, setPosition] = useState<{ left: number; top: number } | null>(null);
  useLayoutEffect(() => {
    const element = box.current;
    if (!element) return;
    const { width, height } = element.getBoundingClientRect();
    const maxLeft = window.innerWidth - width - EDGE;
    const maxTop = window.innerHeight - height - EDGE;
    let left: number;
    let top: number;
    if (isRect(anchor)) {
      left = anchor.left + anchor.width / 2 - width / 2;
      top = anchor.top - height - OFFSET;
      if (top < EDGE) top = anchor.bottom + OFFSET;
    } else {
      left = anchor.clientX + OFFSET;
      top = anchor.clientY + OFFSET;
      if (left > maxLeft) left = anchor.clientX - OFFSET - width;
      if (top > maxTop) top = anchor.clientY - OFFSET - height;
    }
    setPosition({ left: Math.max(EDGE, Math.min(maxLeft, left)), top: Math.max(EDGE, Math.min(maxTop, top)) });
  }, [anchor, children]);
  return (
    <div
      ref={box}
      role="tooltip"
      className="glass pointer-events-none fixed z-popover max-w-72 rounded-lg px-3 py-2 text-caption text-fg"
      style={{ left: position?.left ?? 0, top: position?.top ?? 0, visibility: position ? 'visible' : 'hidden' }}
    >
      {children}
    </div>
  );
}

export interface TooltipRow {
  key: string;
  value: string;
  label: string;
  swatch?: SwatchKind;
}

/** Tooltip body: an optional title, then rows with the value first and its label after. */
export function TooltipContent({ title, rows, note }: { title?: string; rows: readonly TooltipRow[]; note?: string }) {
  return (
    <div className="grid gap-1">
      {title && <div className="font-medium text-fg">{title}</div>}
      {rows.map((row) => (
        <div key={row.key} className="flex items-center gap-2">
          {row.swatch && <LegendSwatch kind={row.swatch} />}
          <span className="viz-num font-semibold text-fg">{row.value}</span>
          <span className="text-fg-secondary">{row.label}</span>
        </div>
      ))}
      {note && <div className="text-muted">{note}</div>}
    </div>
  );
}
