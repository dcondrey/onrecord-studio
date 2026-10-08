import { useLayoutEffect, type DependencyList, type RefObject } from 'react';
import { scrollWithin } from '../utils/scrollWithin';

/**
 * Keeps the element `selector` finds inside `container` scrolled into the container's view: when
 * `deps` change (a new selection, the list opening) and whenever the container is resized (the
 * window shrinking under an open list), so the row a key would act on is never out of sight.
 */
export function useKeepInView(container: RefObject<HTMLElement | null>, selector: string, deps: DependencyList) {
  useLayoutEffect(() => {
    const box = container.current;
    if (!box) return;
    const reveal = () => {
      const target = box.querySelector<HTMLElement>(selector);
      if (target) scrollWithin(box, target, 'nearest', true);
    };
    reveal();
    if (typeof ResizeObserver === 'undefined') return;
    const observer = new ResizeObserver(reveal);
    observer.observe(box);
    return () => observer.disconnect();
    // The caller's deps say when the selection moved; the selector names what to reveal.
  }, [container, selector, ...deps]);
}
