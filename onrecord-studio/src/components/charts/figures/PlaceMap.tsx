import { useMemo } from 'react';
import { formatValue } from '../core/format';
import { measureText } from '../core/layout';
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
  /** Every record, placed or not, as the source query counted it; shares are of this. */
  total: number;
  /** The place boundaries; null while they load. */
  boundaries: readonly PlaceBoundary[] | null;
  error?: string | null;
  sources: ChartSources;
  size: 'card' | 'full';
  /** Activating a place (its circle or its row in the list): opens that place's records. */
  onActivate?(key: string): void;
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

/** Two or three round counts for the size key: the largest, and steps of a quarter of its area. */
function keyCounts(max: number): number[] {
  const round = (v: number) => {
    const magnitude = 10 ** Math.max(0, Math.floor(Math.log10(v)) - 1);
    return Math.max(1, Math.round(v / magnitude) * magnitude);
  };
  return [...new Set([max, round(max / 4), round(max / 16)])].filter((v) => v >= 1 && v <= max).sort((a, b) => b - a);
}

export default function PlaceMap({ title, headline, noun, places, unplaced, total, boundaries, error, sources, size, onActivate }: PlaceMapProps) {
  const tooltip = useTooltip();
  const ranked = useMemo(() => [...places].sort((a, b) => b.count - a.count || a.key.localeCompare(b.key)), [places]);
  const { shapes, unreadable } = useMemo(() => shapesOf(boundaries ?? []), [boundaries]);
  const mapped: MappedPlace[] = useMemo(() => ranked.map((place) => {
    const shape = shapes.get(place.key);
    const at = shape ? centroid(shape) : null;
    return { ...place, at: at ? [at[0], at[1]] : null };
  }), [ranked, shapes]);
  const placedCount = mapped.filter((place) => place.at).length;
  const unmapped = mapped.filter((place) => !place.at);
  const placedKeys = mapped.filter((place) => place.at).map((place) => place.key);
  const roving = useRovingFocus({ shape: { rows: Math.max(1, placedCount) }, onActivate: (i) => { if (placedKeys[i] !== undefined) onActivate?.(placedKeys[i]); }, onEscape: tooltip.hide });

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
      onClick: onActivate ? () => onActivate(place.key) : undefined,
    };
  };

  const height = size === 'card' ? 220 : 340;
  const maxCount = Math.max(0, ...mapped.filter((place) => place.at).map((place) => place.count));
  const radius = useMemo(() => scaleSqrtArea(Math.max(1, maxCount), size === 'card' ? 14 : 22), [maxCount, size]);
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
        legend={[{ key: 'lead', label: 'Most counted', swatch: 'dot', tone: 'lead' }, { key: 'other', label: 'Other places', swatch: 'dot', tone: 'mark' }, { key: 'outline', label: `Outline only: no ${noun} counted`, swatch: 'ring', tone: 'baseline' }]}
        controls={(
          <div className="grid gap-1">
            {maxCount > 0 && <SizeKey counts={keyCounts(maxCount)} radius={radius} noun={noun} />}
            {unplaced > 0 && (
              <p className="w-fit rounded-md border border-line px-2 py-1 text-caption text-fg-secondary">
                <span className="viz-num font-medium text-fg">{unplaced.toLocaleString('en-US')}</span>{` ${noun}: no place recorded (not on the map)`}
              </p>
            )}
          </div>
        )}
        minHeight={height}
        interactive={{ roledescription: 'map' }}
        storageKey={`place-map:${title}`}
      >
        {({ width }) => <MapPlot width={width} height={height} shapes={shapes} outlines={outlines} mapped={mapped} radius={radius} interaction={interaction} />}
      </ChartFrame>
      <RankedList ranked={ranked} total={total} noun={noun} onActivate={onActivate} />
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
function MapPlot({ width, height, shapes, outlines, mapped, radius, interaction }: {
  width: number;
  height: number;
  shapes: Map<string, Geometry>;
  outlines: readonly GeoArea[];
  mapped: readonly MappedPlace[];
  radius(count: number): number;
  interaction(place: MappedPlace, index: number): MarkInteraction;
}) {
  const project = useMemo(() => {
    const bounds = boundsOf([...shapes.values()]);
    return bounds ? equirectangular(bounds, { width, height, padding: 12 }) : null;
  }, [shapes, width, height]);
  if (!project) return null;
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

/** The circles' size key: a few round counts drawn at the map's own radii. */
function SizeKey({ counts, radius, noun }: { counts: readonly number[]; radius(count: number): number; noun: string }) {
  const largest = radius(counts[0]);
  let x = 0;
  const placed = counts.map((count) => {
    const r = radius(count);
    const label = formatValue(count, { kind: 'count' });
    const at = { count, r, cx: x + largest, label, lx: x + largest * 2 + 4 };
    x = at.lx + measureText(label) + 12;
    return at;
  });
  return (
    <div className="flex flex-wrap items-center gap-2 text-caption text-fg-secondary">
      <span>Circle area is proportional to {noun}:</span>
      <svg width={x} height={largest * 2 + 2} aria-hidden="true" className="shrink-0">
        {placed.map((item) => (
          <g key={item.count}>
            <circle cx={item.cx} cy={largest * 2 + 1 - item.r} r={item.r} style={{ fill: 'none', stroke: 'var(--viz-mark)', strokeWidth: 1 }} />
            <text x={item.lx} y={largest + 1} dy="0.32em" className="viz-num fill-fg-secondary text-[11px]">{item.label}</text>
          </g>
        ))}
      </svg>
    </div>
  );
}

/** Every place with a count, ranked, with its share of all records: the map's values without
 * hovering, and a second way to open a place's records. */
function RankedList({ ranked, total, noun, onActivate }: { ranked: readonly PlaceCount[]; total: number; noun: string; onActivate?(key: string): void }) {
  if (!ranked.length) return null;
  const max = Math.max(1, ranked[0].count);
  return (
    <div className="grid gap-1">
      <p className="text-caption text-muted">{`Every place with ${noun}, ranked (${ranked.length})`}</p>
      <ol aria-label={`Places by ${noun}`} className="max-h-40 space-y-0.5 overflow-y-auto pr-1 text-caption">
        {ranked.map((place, index) => {
          const body = (
            <>
              <span className="w-6 shrink-0 text-right tabular-nums text-muted">{index + 1}</span>
              <span className="min-w-0 flex-1 truncate text-fg-secondary">{place.label}</span>
              <span aria-hidden="true" className="h-2 w-16 shrink-0 overflow-hidden rounded-sm bg-[var(--viz-track)]">
                <span className="block h-full" style={{ width: `${(place.count / max) * 100}%`, background: index === 0 ? 'var(--viz-lead)' : 'var(--viz-mark)' }} />
              </span>
              <span className="w-24 shrink-0 text-right tabular-nums text-fg">{`${formatValue(place.count, { kind: 'count' })}${total > 0 ? ` (${formatValue(place.count / total, { kind: 'percent' })})` : ''}`}</span>
            </>
          );
          return (
            <li key={place.key}>
              {onActivate ? (
                <button type="button" className="focus-ring flex w-full items-center gap-2 rounded px-1 py-0.5 text-left hover:bg-raised" onClick={() => onActivate(place.key)}>{body}</button>
              ) : <div className="flex items-center gap-2 px-1 py-0.5">{body}</div>}
            </li>
          );
        })}
      </ol>
    </div>
  );
}
