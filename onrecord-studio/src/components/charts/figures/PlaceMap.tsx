import { useMemo } from 'react';
import { formatValue } from '../core/format';
import { boundsOf, centroid, equirectangular, parseFeature, type Geometry, type PolygonCoordinates } from '../core/geo';
import { scaleSqrtArea } from '../core/scale';
import ChartFrame, { type ChartStatus } from '../frame/ChartFrame';
import type { ChartSources } from '../frame/SourceCaption';
import type { TwinTable } from '../frame/TableTwin';
import { TooltipContent } from '../frame/Tooltip';
import { useRovingFocus } from '../hooks/useRovingFocus';
import { useTooltip } from '../hooks/useTooltip';
import { GeoCircles, GeoOutlines, type GeoArea } from '../marks/GeoLayer';
import type { MarkInteraction } from '../marks/shared';

/*
 * Counts per place as proportional circles at each place's centroid over its outline, not a
 * choropleth: a raw count per area mostly shows its size and population. A record with no place
 * is a tile beside the map, never placed. A place in the boundary file with no count is an
 * outline only, which says no record was counted there, not that the count is zero.
 */

export interface PlaceCount {
  /** The place's code as both files write it (beat '521'). */
  key: string;
  label: string;
  count: number;
}

export interface PlaceBoundary {
  key: string;
  /** The stored feature row: `_geometry` JSON and `_crs`. */
  _geometry: unknown;
  _crs?: unknown;
}

export interface PlaceMapProps {
  title: string;
  headline: string | null;
  /** What is counted, plural ('SLEEPER calls'). */
  noun: string;
  /** Every place with a count: a full group, never a top-N list. */
  places: readonly PlaceCount[];
  /** Records with no place recorded. */
  unplaced: number;
  /** The place boundaries; null while they load. */
  boundaries: readonly PlaceBoundary[] | null;
  error?: string | null;
  sources: ChartSources;
  size: 'card' | 'full';
}

interface MappedPlace extends PlaceCount {
  at: [number, number] | null;
}

/** Each place's boundary parts merged into one shape, and the parts no projection can read. */
function shapesOf(boundaries: readonly PlaceBoundary[]): { shapes: Map<string, Geometry>; unreadable: number } {
  const parts = new Map<string, PolygonCoordinates[]>();
  let unreadable = 0;
  for (const boundary of boundaries) {
    const feature = parseFeature(boundary);
    if ('unsupported' in feature) {
      unreadable += 1;
      continue;
    }
    const polygons = feature.geometry.type === 'Polygon' ? [feature.geometry.coordinates] : feature.geometry.type === 'MultiPolygon' ? feature.geometry.coordinates : [];
    parts.set(boundary.key, [...(parts.get(boundary.key) ?? []), ...polygons]);
  }
  return { shapes: new Map([...parts].map(([key, polygons]) => [key, { type: 'MultiPolygon', coordinates: polygons } as Geometry])), unreadable };
}

export default function PlaceMap({ title, headline, noun, places, unplaced, boundaries, error, sources, size }: PlaceMapProps) {
  const tooltip = useTooltip();
  const ranked = useMemo(() => [...places].sort((a, b) => b.count - a.count || a.key.localeCompare(b.key)), [places]);
  const total = ranked.reduce((sum, place) => sum + place.count, 0) + unplaced;
  const { shapes, unreadable } = useMemo(() => shapesOf(boundaries ?? []), [boundaries]);
  const mapped: MappedPlace[] = useMemo(() => ranked.map((place) => {
    const shape = shapes.get(place.key);
    const at = shape ? centroid(shape) : null;
    return { ...place, at: at ? [at[0], at[1]] : null };
  }), [ranked, shapes]);
  const placedCount = mapped.filter((place) => place.at).length;
  const unmapped = mapped.filter((place) => !place.at);
  const roving = useRovingFocus({ shape: { rows: Math.max(1, placedCount) }, onActivate: () => {}, onEscape: tooltip.hide });

  const status: ChartStatus = error ? 'error' : boundaries === null ? 'loading' : shapes.size === 0 ? 'empty' : 'ready';
  const share = (count: number) => (total > 0 ? formatValue(count / total, { kind: 'percent' }) : '');
  const table: TwinTable = {
    caption: `${title}, every place with a count`,
    columns: [{ key: 'place', label: 'Place' }, { key: 'count', label: noun[0].toUpperCase() + noun.slice(1), kind: 'count' }, { key: 'share', label: 'Share of all' }, { key: 'mapped', label: 'On the map' }],
    rows: [
      ...mapped.map((place) => ({ place: place.label, count: place.count, share: share(place.count), mapped: place.at ? 'yes' : 'no boundary in the file' })),
      ...(unplaced ? [{ place: 'No place recorded', count: unplaced, share: share(unplaced), mapped: 'not placed' }] : []),
    ],
  };

  const interaction = (place: MappedPlace, index: number): MarkInteraction => {
    const content = <TooltipContent title={place.label} rows={[{ key: 'n', value: formatValue(place.count, { kind: 'count' }), label: noun }, { key: 's', value: share(place.count), label: 'of all' }]} />;
    return {
      name: `${place.label}: ${formatValue(place.count, { kind: 'count' })} ${noun}, ${share(place.count)} of all.`,
      item: roving.getItemProps(index),
      onPointerMove: (event) => tooltip.show(content, { clientX: event.clientX, clientY: event.clientY }),
      onPointerLeave: tooltip.hide,
      onFocusMark: (box) => tooltip.show(content, box),
      onBlurMark: tooltip.hide,
    };
  };

  const height = size === 'card' ? 220 : 340;
  const counted = useMemo(() => new Set(places.map((place) => place.key)), [places]);
  const outlines: GeoArea[] = useMemo(() => [...shapes].map(([key, geometry]) => ({ key, geometry, title: counted.has(key) ? key : `${key}: no ${noun} counted` })), [shapes, counted, noun]);

  return (
    <div className="grid min-w-0 gap-2" data-place-map="">
      <ChartFrame
        title={title}
        headline={headline}
        claim="descriptive"
        sources={sources}
        summary={`${title}. ${placedCount} places drawn as circles sized by count; ${unplaced.toLocaleString('en-US')} ${noun} with no place recorded are not placed.`}
        status={status}
        error={error}
        emptyText="The boundary file has no shape this map can read."
        table={table}
        legend={[{ key: 'lead', label: 'Most counted', swatch: 'dot', tone: 'lead' }, { key: 'other', label: 'Other places (area ∝ count)', swatch: 'dot', tone: 'mark' }, { key: 'outline', label: `Outline only: no ${noun} counted`, swatch: 'ring', tone: 'baseline' }]}
        controls={unplaced > 0 ? (
          <p className="rounded-md border border-line px-2 py-1 text-caption text-fg-secondary">
            <span className="viz-num font-medium text-fg">{unplaced.toLocaleString('en-US')}</span>{` ${noun}: no place recorded (not on the map)`}
          </p>
        ) : undefined}
        minHeight={height}
        interactive={{ roledescription: 'map' }}
        storageKey={`place-map:${title}`}
      >
        {({ width }) => <MapPlot width={width} height={height} shapes={shapes} outlines={outlines} mapped={mapped} size={size} interaction={interaction} />}
      </ChartFrame>
      {(unmapped.length > 0 || unreadable > 0) && (
        <p className="text-caption text-muted">
          {[unmapped.length ? `${unmapped.length} ${unmapped.length === 1 ? 'place with a count has' : 'places with counts have'} no boundary in the file (${unmapped.slice(0, 5).map((place) => place.label).join(', ')}${unmapped.length > 5 ? ', …' : ''}).` : '',
            unreadable ? `${unreadable} boundary ${unreadable === 1 ? 'part' : 'parts'} could not be read and are not drawn.` : ''].filter(Boolean).join(' ')}
        </p>
      )}
      {tooltip.node}
    </div>
  );
}

/** The plot at one width. The projection and the outlines are kept while only the tooltip changes,
 * so a pointer move never re-projects every boundary vertex. */
function MapPlot({ width, height, shapes, outlines, mapped, size, interaction }: {
  width: number;
  height: number;
  shapes: Map<string, Geometry>;
  outlines: readonly GeoArea[];
  mapped: readonly MappedPlace[];
  size: 'card' | 'full';
  interaction(place: MappedPlace, index: number): MarkInteraction;
}) {
  const project = useMemo(() => {
    const bounds = boundsOf([...shapes.values()]);
    return bounds ? equirectangular(bounds, { width, height, padding: 12 }) : null;
  }, [shapes, width, height]);
  if (!project) return null;
  const radius = scaleSqrtArea(Math.max(1, ...mapped.map((place) => place.count)), size === 'card' ? 14 : 22);
  let index = 0;
  return (
    <g>
      <GeoOutlines areas={outlines} project={project} />
      <GeoCircles
        circles={mapped.flatMap((place, rank) => {
          if (!place.at) return [];
          const [cx, cy] = project(place.at[0], place.at[1]);
          return [{ key: place.key, cx, cy, r: radius(place.count), role: rank === 0 ? 'lead' as const : 'default' as const, interaction: interaction(place, index++) }];
        })}
      />
    </g>
  );
}
