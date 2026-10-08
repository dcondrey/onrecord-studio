import { useCallback, useId, useState, type CSSProperties } from 'react';
import { withViewTransition } from '../../../utils/viewTransition';

type ViewTransitionDocument = Document & { startViewTransition?: unknown };

function transitionAvailable(): boolean {
  return typeof document !== 'undefined'
    && typeof (document as ViewTransitionDocument).startViewTransition === 'function'
    && document.documentElement.dataset.motion !== 'reduce';
}

/**
 * One expanded row at a time. Where WebKit has view transitions (18+) and motion is full, the
 * toggle runs as one, with the row named `viz-row-<key>`; otherwise (macOS 13 and 14, or reduced
 * motion) the state changes synchronously and the region opens on a CSS grid-rows transition that
 * the reduce setting makes instant.
 */
export function useDrillDown() {
  const [expanded, setExpanded] = useState<string | null>(null);
  const base = useId().replace(/[^a-zA-Z0-9_-]/g, '');

  const toggle = useCallback((key: string) => {
    withViewTransition(() => setExpanded((current) => (current === key ? null : key)));
  }, []);

  const safe = (key: string) => `${base}-${key.replace(/[^a-zA-Z0-9_-]/g, '_')}`;
  const regionId = (key: string) => `viz-drill-${safe(key)}`;

  return {
    expanded,
    toggle,
    // Named per hook instance as well as per key: a duplicate view-transition-name on the page
    // aborts the transition.
    rowProps: (key: string): { style: CSSProperties } => ({ style: { viewTransitionName: `viz-row-${safe(key)}` } as CSSProperties }),
    buttonProps: (key: string) => ({ 'aria-expanded': expanded === key, 'aria-controls': regionId(key), onClick: () => toggle(key) }),
    regionProps: (key: string) => ({
      id: regionId(key),
      className: 'viz-drill',
      'data-open': expanded === key ? '' : undefined,
      'data-instant': transitionAvailable() ? '' : undefined,
    }),
  };
}
