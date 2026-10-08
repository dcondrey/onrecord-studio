import { useLayoutEffect, useState, type RefObject } from 'react';

export interface ChartSize {
  width: number;
  height: number;
}

/**
 * The measured content box of a plot container, in integer px. Resize notifications are coalesced
 * to one per animation frame. Width is 0 until the first measure, and the caller renders no marks
 * until then; the frame reserves its min-height so nothing jumps. `fallbackWidth` stands in where
 * there is no ResizeObserver (jsdom).
 */
export function useChartSize(ref: RefObject<HTMLElement | null>, { fallbackWidth }: { fallbackWidth?: number } = {}): ChartSize {
  const [size, setSize] = useState<ChartSize>({ width: 0, height: 0 });
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    if (typeof ResizeObserver === 'undefined') {
      setSize({ width: Math.round(element.clientWidth || fallbackWidth || 0), height: Math.round(element.clientHeight) });
      return;
    }
    let frame = 0;
    const measure = () => {
      frame = 0;
      const next = { width: Math.round(element.clientWidth), height: Math.round(element.clientHeight) };
      setSize((current) => (current.width === next.width && current.height === next.height ? current : next));
    };
    measure();
    const observer = new ResizeObserver(() => {
      if (!frame) frame = requestAnimationFrame(measure);
    });
    observer.observe(element);
    return () => {
      observer.disconnect();
      if (frame) cancelAnimationFrame(frame);
    };
  }, [ref, fallbackWidth]);
  return size;
}
