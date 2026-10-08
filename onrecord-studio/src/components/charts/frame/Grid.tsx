import { VIZ } from '../core/claim';
import type { ContinuousScale } from '../core/scale';

/**
 * Solid 1px gridlines at the scale's ticks, never dashed. `vertical` draws lines across x (for a
 * horizontal value axis); otherwise lines across y.
 */
export function Grid({ scale, extent, vertical = false, tickCount }: {
  scale: ContinuousScale;
  /** The cross-axis span the lines cover, px. */
  extent: readonly [number, number];
  vertical?: boolean;
  tickCount?: number;
}) {
  const length = Math.abs(scale.range[1] - scale.range[0]);
  const ticks = scale.ticks(tickCount ?? Math.max(2, Math.floor(length / 72)));
  const style = { stroke: VIZ.grid, strokeWidth: 1, shapeRendering: 'crispEdges' as const };
  return (
    <g aria-hidden="true">
      {ticks.map((tick) => {
        const at = Math.round(scale(tick)) + 0.5;
        return vertical
          ? <line key={tick} x1={at} x2={at} y1={extent[0]} y2={extent[1]} style={style} />
          : <line key={tick} x1={extent[0]} x2={extent[1]} y1={at} y2={at} style={style} />;
      })}
    </g>
  );
}
