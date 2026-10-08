import { useCallback, useMemo, useState } from 'react';

/** Opacity of a series that is not the focused one. Colour never changes. */
export const DIM_OPACITY = 0.15;

/**
 * Legend focus: hovering an entry spotlights its series for as long as the pointer stays; clicking
 * makes it stick; clicking it again or Escape clears it.
 */
export function useLegendFocus() {
  const [sticky, setSticky] = useState<string | null>(null);
  const [hovered, setHovered] = useState<string | null>(null);
  const focused = hovered ?? sticky;
  const hover = useCallback((key: string | null) => setHovered(key), []);
  const toggle = useCallback((key: string) => setSticky((current) => (current === key ? null : key)), []);
  const clear = useCallback(() => {
    setSticky(null);
    setHovered(null);
  }, []);
  const opacityFor = useCallback((key: string) => (focused === null || focused === key ? 1 : DIM_OPACITY), [focused]);
  return useMemo(() => ({ focused, sticky, hover, toggle, clear, opacityFor }), [focused, sticky, hover, toggle, clear, opacityFor]);
}

export type LegendFocus = ReturnType<typeof useLegendFocus>;
