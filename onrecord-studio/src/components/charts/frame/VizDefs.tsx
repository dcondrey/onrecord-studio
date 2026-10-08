import { createContext, useContext, useId } from 'react';

/** An SVG id from React's useId, which contains colons that some WebKit builds reject in url(#…). */
export function useSvgId(prefix: string): string {
  return `${prefix}-${useId().replace(/[^a-zA-Z0-9_-]/g, '')}`;
}

const HatchContext = createContext<string | null>(null);

/**
 * The one 45° hatch, reserved for the 'excluded' absence state: real records outside a measure's
 * denominator. ChartFrame renders it once per plot and provides its id to the marks inside.
 */
export function VizDefs({ hatchId }: { hatchId: string }) {
  return (
    <defs>
      <pattern id={hatchId} width={5} height={5} patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
        <line x1={0} y1={0} x2={0} y2={5} style={{ stroke: 'var(--viz-mark)', strokeWidth: 1.5 }} />
      </pattern>
    </defs>
  );
}

export const HatchProvider = HatchContext.Provider;

/** `url(#…)` of the enclosing plot's hatch. A mark outside a plot has no hatch to draw. */
export function useHatchFill(): string {
  const id = useContext(HatchContext);
  if (!id) throw new Error('A hatched mark must render inside a ChartFrame plot or a HatchProvider.');
  return `url(#${id})`;
}
