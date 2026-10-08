import { createContext, useCallback, useContext, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react';

function readReduce(): boolean {
  return typeof document === 'undefined' || document.documentElement.dataset.motion !== 'full';
}

/**
 * Chart motion follows the app's resolved setting, `:root[data-motion]` (the in-app preference over
 * the OS query), not the media query alone. A mark animates the first time its key is committed:
 * bars grow from the baseline and lines draw, once. A data refresh re-renders the same keys and
 * never animates; under reduce nothing moves.
 */
export function useChartMotion(): { reduce: boolean; enterOnce(key: string): boolean } {
  const [reduce, setReduce] = useState(readReduce);
  const committed = useRef(new Set<string>());
  const pending = useRef(new Set<string>());

  useEffect(() => {
    if (typeof MutationObserver === 'undefined') return;
    const observer = new MutationObserver(() => setReduce(readReduce()));
    observer.observe(document.documentElement, { attributes: true, attributeFilter: ['data-motion'] });
    return () => observer.disconnect();
  }, []);

  useLayoutEffect(() => {
    for (const key of pending.current) committed.current.add(key);
    pending.current.clear();
  });

  const enterOnce = useCallback((key: string) => {
    if (committed.current.has(key)) return false;
    pending.current.add(key);
    return !reduce;
  }, [reduce]);

  return useMemo(() => ({ reduce, enterOnce }), [reduce, enterOnce]);
}

export type ChartMotion = ReturnType<typeof useChartMotion>;

const STILL: ChartMotion = { reduce: true, enterOnce: () => false };

/** One motion state per frame, shared by its marks. A mark outside a frame never animates. */
export const ChartMotionContext = createContext<ChartMotion>(STILL);

export function useFrameMotion(): ChartMotion {
  return useContext(ChartMotionContext);
}
