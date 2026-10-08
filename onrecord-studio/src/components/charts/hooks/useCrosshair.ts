import { useState, type KeyboardEvent, type PointerEvent } from 'react';
import { useSyncedKey } from './ChartSync';

/** Index of the value in a sorted array nearest to `x` (binary search). */
export function nearestIndex(xs: readonly number[], x: number): number {
  if (xs.length === 0) return -1;
  let lo = 0;
  let hi = xs.length - 1;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (xs[mid] < x) lo = mid + 1;
    else hi = mid;
  }
  return lo > 0 && x - xs[lo - 1] <= xs[lo] - x ? lo - 1 : lo;
}

export interface CrosshairOptions {
  /** Sorted x positions in px, relative to the bound element's left edge. */
  xs: readonly number[];
  /** One key per position; shared with the sync group. */
  keys: readonly string[];
  syncGroup?: string;
}

/**
 * A vertical crosshair snapped to the nearest observed x. Pointer moves set it; Left and Right
 * move it from the keyboard; Escape clears it. Inside a ChartSync with a `syncGroup`, the active
 * key is shared with every panel in the group.
 */
export function useCrosshair({ xs, keys, syncGroup }: CrosshairOptions) {
  const [local, setLocal] = useState<number | null>(null);
  const [syncedKey, setSyncedKey] = useSyncedKey(syncGroup);
  const synced = setSyncedKey ? (syncedKey === null ? null : keys.indexOf(syncedKey)) : undefined;
  const index = synced === undefined ? local : synced === -1 ? null : synced;

  const setIndex = (next: number | null) => {
    const value = next === null || xs.length === 0 ? null : Math.max(0, Math.min(xs.length - 1, next));
    if (setSyncedKey) setSyncedKey(value === null ? null : keys[value]);
    else setLocal(value);
  };

  const bind = {
    onPointerMove(event: PointerEvent<Element>) {
      const box = event.currentTarget.getBoundingClientRect();
      setIndex(nearestIndex(xs, event.clientX - box.left));
    },
    onPointerLeave() {
      setIndex(null);
    },
    onKeyDown(event: KeyboardEvent<Element>) {
      if (event.key === 'ArrowRight' || event.key === 'ArrowLeft') {
        event.preventDefault();
        const step = event.key === 'ArrowRight' ? 1 : -1;
        setIndex(index === null ? (step > 0 ? 0 : xs.length - 1) : index + step);
      } else if (event.key === 'Escape') {
        setIndex(null);
      }
    },
  };

  return { index, bind, setIndex };
}
