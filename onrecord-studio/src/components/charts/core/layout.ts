import type { ContinuousScale } from './scale';

/** The chart label font: the app's system stack at the caption size. */
export const LABEL_FONT = '11px -apple-system, BlinkMacSystemFont, "SF Pro Text", "Segoe UI", Roboto, sans-serif';

type TextContext = { font: string; measureText(text: string): { width: number } };

let context: TextContext | null | undefined;
const widths = new Map<string, number>();

function textContext(): TextContext | null {
  if (context !== undefined) return context;
  context = null;
  try {
    if (typeof OffscreenCanvas !== 'undefined') {
      context = new OffscreenCanvas(1, 1).getContext('2d') as TextContext | null;
    } else if (typeof document !== 'undefined' && !/jsdom/i.test(globalThis.navigator?.userAgent ?? '')) {
      context = document.createElement('canvas').getContext('2d');
    }
  } catch {
    context = null;
  }
  return context;
}

/**
 * Rendered width of a label in px, from a cached canvas context. Where there is no canvas (tests),
 * a per-character estimate that errs wide, so a label judged to fit does fit.
 */
export function measureText(text: string, font = LABEL_FONT): number {
  const key = `${font}\u0000${text}`;
  const cached = widths.get(key);
  if (cached !== undefined) return cached;
  const ctx = textContext();
  let width: number;
  if (ctx) {
    ctx.font = font;
    width = ctx.measureText(text).width;
  } else {
    const size = Number(/(\d+(?:\.\d+)?)px/.exec(font)?.[1] ?? 11);
    width = text.length * size * 0.62;
  }
  if (widths.size > 4000) widths.clear();
  widths.set(key, width);
  return width;
}

/** Whether a label fits inside a mark with 4px padding each side. One that does not moves outside. */
export function fitsInside(text: string, widthPx: number, font = LABEL_FONT): boolean {
  return measureText(text, font) + 8 <= widthPx;
}

export interface SwarmPoint {
  key: string;
  cx: number;
  /** Offset from the strip's centre line, px. */
  dy: number;
}

/**
 * A deterministic beeswarm: points are placed in (value, key) order, each at the first row offset
 * (0, +1, −1, +2, …) where it overlaps nothing already placed. Computed once, never simulated, so
 * the same data always lands in the same place and there is nothing to animate. Past `maxRows` on
 * either side a point stays on the outermost row (overlapping), so it is never dropped.
 */
export function beeswarm(items: readonly { key: string; value: number }[], x: ContinuousScale, r: number, maxRows: number): SwarmPoint[] {
  const sorted = [...items].sort((a, b) => a.value - b.value || (a.key < b.key ? -1 : a.key > b.key ? 1 : 0));
  const diameter = 2 * r + 1;
  const placed: SwarmPoint[] = [];
  for (const item of sorted) {
    const cx = x(item.value);
    const near = placed.filter((p) => Math.abs(p.cx - cx) < diameter);
    let chosen = 0;
    let found = false;
    for (let step = 0; step <= 2 * maxRows; step++) {
      const row = step === 0 ? 0 : step % 2 === 1 ? (step + 1) / 2 : -step / 2;
      const dy = row * diameter;
      if (near.every((p) => Math.hypot(p.cx - cx, p.dy - dy) >= diameter)) {
        chosen = dy;
        found = true;
        break;
      }
    }
    if (!found) chosen = maxRows * diameter;
    placed.push({ key: item.key, cx, dy: chosen });
  }
  return placed;
}

export interface PlacedLabel {
  key: string;
  /** Start position of the label along the axis. */
  pos: number;
  /** The label moved far enough from its anchor to need a leader line. */
  leader: boolean;
}

/**
 * One-dimensional label placement: each label is centred on its anchor, then pushed apart so no
 * two overlap (with `gap` between), and the whole run is kept inside [min, max]. Returned in the
 * input order.
 */
export function placeLabels(labels: readonly { key: string; anchor: number; width: number }[], min: number, max: number, gap = 4): PlacedLabel[] {
  const order = labels.map((label, index) => ({ ...label, index })).sort((a, b) => a.anchor - b.anchor || a.index - b.index);
  const pos = order.map((label) => label.anchor - label.width / 2);
  for (let i = 0; i < order.length; i++) {
    const floor = i === 0 ? min : pos[i - 1] + order[i - 1].width + gap;
    pos[i] = Math.max(pos[i], floor);
  }
  for (let i = order.length - 1; i >= 0; i--) {
    const ceiling = i === order.length - 1 ? max - order[i].width : pos[i + 1] - gap - order[i].width;
    pos[i] = Math.min(pos[i], ceiling);
  }
  for (let i = 0; i < order.length; i++) pos[i] = Math.max(pos[i], i === 0 ? min : pos[i - 1] + order[i - 1].width + gap);
  const result = new Array<PlacedLabel>(labels.length);
  order.forEach((label, i) => {
    const centre = pos[i] + label.width / 2;
    result[label.index] = { key: label.key, pos: pos[i], leader: Math.abs(centre - label.anchor) > label.width / 2 };
  });
  return result;
}
