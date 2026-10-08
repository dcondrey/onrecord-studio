import { memo } from 'react';
import { paintFor, VIZ } from '../core/claim';
import { geoPath, type Geometry, type Projection } from '../core/geo';
import { MarkGroup, type MarkInteraction } from './shared';

export interface GeoArea {
  key: string;
  geometry: Geometry;
  /** Named in the area's <title>: 'Beat 521 (East Village): no calls counted'. */
  title: string;
}

/**
 * Area outlines with no fill: a base map with no tiles. `weight` 'hairline' is the receding
 * baseline stroke (beats); 'outline' a heavier context stroke for an overlay (council districts).
 */
export const GeoOutlines = memo(function GeoOutlines({ areas, project, weight = 'hairline' }: { areas: readonly GeoArea[]; project: Projection; weight?: 'hairline' | 'outline' }) {
  const style = weight === 'hairline'
    ? { fill: 'none', stroke: VIZ.baseline, strokeWidth: 0.75 }
    : { fill: 'none', stroke: VIZ.context, strokeWidth: 1.5 };
  return (
    <g data-geo={weight}>
      {areas.map((area) => (
        <path key={area.key} d={geoPath(area.geometry, project)} style={{ ...style, strokeLinejoin: 'round' }}>
          <title>{area.title}</title>
        </path>
      ))}
    </g>
  );
});

export interface GeoCircle {
  key: string;
  cx: number;
  cy: number;
  /** Area-proportional radius (scaleSqrtArea). */
  r: number;
  /** The lead's own place is violet; every other place is the default mark at 60%. */
  role: 'lead' | 'default';
  interaction?: MarkInteraction;
}

/**
 * Proportional circles at area centroids, largest first so a small circle is never hidden under a
 * large one, each with a 2px surface ring.
 */
export function GeoCircles({ circles }: { circles: readonly GeoCircle[] }) {
  const ordered = [...circles].sort((a, b) => b.r - a.r || a.key.localeCompare(b.key));
  return (
    <g data-geo="circles">
      {ordered.map((circle) => {
        const paint = paintFor('descriptive', 'fill', circle.role);
        const r = Math.max(circle.r, 1.5);
        return (
          <MarkGroup key={circle.key} interaction={circle.interaction} box={{ x: circle.cx - r, y: circle.cy - r, width: 2 * r, height: 2 * r }} ringRadius={r + 3}>
            <circle cx={circle.cx} cy={circle.cy} r={r} style={{ fill: paint.fill, fillOpacity: circle.role === 'lead' ? 1 : 0.6, stroke: VIZ.surface, strokeWidth: 2, paintOrder: 'stroke' }} />
          </MarkGroup>
        );
      })}
    </g>
  );
}
