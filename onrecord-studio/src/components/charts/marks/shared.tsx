import type { FocusEvent, PointerEvent, ReactNode } from 'react';
import { VIZ } from '../core/claim';
import { ABSENCE_LABEL, type Absence } from '../core/nulls';
import { fitsInside } from '../core/layout';
import type { RovingItemProps } from '../hooks/useRovingFocus';
import { useHatchFill } from '../frame/VizDefs';

/** Every mark's hit area is at least this big, whatever the mark's drawn size. */
export const MIN_HIT = 24;
export const LABEL_CLASS = 'viz-num fill-fg-secondary text-[11px]';
export const MUTED_LABEL_CLASS = 'fill-muted text-[11px]';

export interface MarkInteraction {
  /** A full sentence: the mark's accessible name ('Police: 22 of 268 requests missed the deadline'). */
  name: string;
  /** Roving-focus props from useRovingFocus().getItemProps. */
  item?: RovingItemProps;
  onPointerMove?(event: PointerEvent<SVGGElement>): void;
  onPointerLeave?(): void;
  /** Keyboard focus reached the mark: anchor a tooltip to its box. */
  onFocusMark?(box: DOMRect): void;
  onBlurMark?(): void;
  onClick?(): void;
  /** `button` for a mark whose activation opens what it names; default `img`. */
  role?: 'button';
}

export interface Box {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Grows a box about its centre to the minimum hit size. */
export function hitBox(box: Box): Box {
  const width = Math.max(box.width, MIN_HIT);
  const height = Math.max(box.height, MIN_HIT);
  return { x: box.x + (box.width - width) / 2, y: box.y + (box.height - height) / 2, width, height };
}

/**
 * Wraps a mark: an invisible hit area of at least 24px, the focus ring (focus colour, never violet),
 * and the accessible name when the mark is interactive. A non-interactive mark is presentational.
 */
export function MarkGroup({ interaction, box, ringRadius = 3, children }: { interaction?: MarkInteraction; box: Box; ringRadius?: number; children: ReactNode }) {
  if (!interaction) return <g>{children}</g>;
  const hit = hitBox(box);
  const { item } = interaction;
  const button = interaction.role === 'button';
  // The control is the element focus lands on. A button is the hit area itself, beside the marks:
  // its name is the full sentence, and the figures drawn in the row are not text inside it.
  const control = {
    role: interaction.role ?? 'img',
    'aria-label': interaction.name,
    tabIndex: item ? item.tabIndex : undefined,
    ref: item?.ref,
    onKeyDown: item?.onKeyDown,
    onFocus: (event: FocusEvent<SVGElement>) => {
      item?.onFocus();
      interaction.onFocusMark?.((button ? event.currentTarget.parentElement ?? event.currentTarget : event.currentTarget).getBoundingClientRect());
    },
    onBlur: () => {
      item?.onBlur();
      interaction.onBlurMark?.();
    },
  };
  const hitArea = <rect x={hit.x} y={hit.y} width={hit.width} height={hit.height} fill="transparent" pointerEvents="all" className={button ? 'outline-none' : undefined} {...(button ? control : {})} />;
  return (
    <g
      {...(button ? {} : control)}
      className="viz-focusable"
      data-focused={item?.['data-focused']}
      onPointerMove={interaction.onPointerMove}
      onPointerLeave={interaction.onPointerLeave}
      onClick={interaction.onClick}
      style={interaction.onClick ? { cursor: 'pointer' } : undefined}
    >
      {hitArea}
      {children}
      <rect
        className="viz-focus-ring"
        x={box.x - 3}
        y={box.y - 3}
        width={box.width + 6}
        height={box.height + 6}
        rx={ringRadius}
        style={{ fill: 'none', stroke: VIZ.focus, strokeWidth: 2 }}
        pointerEvents="none"
      />
    </g>
  );
}

/**
 * A horizontal bar path from the baseline `x0` to the data end `x1` (either side of it): square at
 * the baseline, 4px rounded at the data end. Drawn as a path because rx would round both ends.
 */
export function hBarPath(x0: number, x1: number, y: number, h: number, radius = 4): string {
  const length = Math.abs(x1 - x0);
  const r = Math.min(radius, length, h / 2);
  if (x1 >= x0) {
    return `M${x0},${y}H${x1 - r}A${r},${r} 0 0 1 ${x1},${y + r}V${y + h - r}A${r},${r} 0 0 1 ${x1 - r},${y + h}H${x0}Z`;
  }
  return `M${x0},${y}H${x1 + r}A${r},${r} 0 0 0 ${x1},${y + r}V${y + h - r}A${r},${r} 0 0 0 ${x1 + r},${y + h}H${x0}Z`;
}

/** A vertical column path from the baseline `y0` up (or down) to the data end `y1`. */
export function vBarPath(y0: number, y1: number, x: number, w: number, radius = 4): string {
  const length = Math.abs(y1 - y0);
  const r = Math.min(radius, length, w / 2);
  if (y1 <= y0) {
    return `M${x},${y0}V${y1 + r}A${r},${r} 0 0 1 ${x + r},${y1}H${x + w - r}A${r},${r} 0 0 1 ${x + w},${y1 + r}V${y0}Z`;
  }
  return `M${x},${y0}V${y1 - r}A${r},${r} 0 0 0 ${x + r},${y1}H${x + w - r}A${r},${r} 0 0 0 ${x + w},${y1 - r}V${y0}Z`;
}

const SLOT_DASH: Record<Absence['state'], string | undefined> = {
  'not-recorded': '3 2',
  'not-covered': '1 2',
  excluded: undefined,
};

/** Short slot text per absence state; the full reason is the slot's <title>. */
export const SLOT_TEXT: Record<Absence['state'], string> = {
  'not-recorded': 'not recorded',
  'not-covered': 'not checked',
  excluded: 'outside this measure',
};

/**
 * The would-be footprint of an absent value: a dashed outline (not recorded), a dotted outline
 * (not checked), or a hatched stub (excluded), with its words inside when they fit, else after it.
 */
export function AbsentSlot({ absence, box, text, align = 'start' }: { absence: Absence; box: Box; text?: string; align?: 'start' | 'end' }) {
  const hatch = useHatchFill();
  const words = text ?? SLOT_TEXT[absence.state];
  // A tall, narrow slot (a column) carries its words rotated inside, so they never run into the
  // next column.
  const upright = box.height > box.width * 2 && fitsInside(words, box.height) && box.width >= 12;
  const inside = fitsInside(words, box.width) && box.height >= 12;
  return (
    <g data-absence={absence.state}>
      <title>{`${ABSENCE_LABEL[absence.state]}: ${absence.reason}`}</title>
      <rect
        x={box.x + 0.5}
        y={box.y + 0.5}
        width={Math.max(0, box.width - 1)}
        height={Math.max(0, box.height - 1)}
        rx={2}
        style={{
          fill: absence.state === 'excluded' ? hatch : 'none',
          stroke: VIZ.baseline,
          strokeWidth: 1,
          strokeDasharray: SLOT_DASH[absence.state],
        }}
      />
      {upright ? (
        <text
          x={box.x + box.width / 2}
          y={box.y + box.height / 2}
          dy="0.32em"
          textAnchor="middle"
          transform={`rotate(-90 ${box.x + box.width / 2} ${box.y + box.height / 2})`}
          className={MUTED_LABEL_CLASS}
        >
          {words}
        </text>
      ) : box.height >= 10 && (
        <text
          x={inside ? (align === 'end' ? box.x + box.width - 4 : box.x + 4) : box.x + box.width + 4}
          y={box.y + box.height / 2}
          dy="0.32em"
          textAnchor={inside && align === 'end' ? 'end' : 'start'}
          className={MUTED_LABEL_CLASS}
          style={inside && absence.state === 'excluded' ? { paintOrder: 'stroke', stroke: VIZ.surface, strokeWidth: 3 } : undefined}
        >
          {words}
        </text>
      )}
    </g>
  );
}
