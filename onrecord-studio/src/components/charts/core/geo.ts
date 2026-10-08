/**
 * Hand-rolled geography for place maps. There is no spatial extension (the app never loads a
 * DuckDB extension), and at city scale an equirectangular projection scaled by cos(latitude) is
 * accurate to well under a pixel. Only longitude/latitude in degrees is read: GeoJSON's default
 * (RFC 7946, CRS84) or a stated EPSG:4326. Any other reference system is refused, never guessed.
 */

export type Position = readonly [number, number];
export type Ring = readonly Position[];
export type PolygonCoordinates = readonly Ring[];

export type Geometry =
  | { type: 'Polygon'; coordinates: PolygonCoordinates }
  | { type: 'MultiPolygon'; coordinates: readonly PolygonCoordinates[] }
  | { type: 'LineString'; coordinates: Ring }
  | { type: 'Point'; coordinates: Position };

export interface Feature {
  geometry: Geometry;
  vertices: number;
}

/** A feature with more vertices than this is refused rather than drawn slowly or partly. */
export const MAX_FEATURE_VERTICES = 200_000;

const LONLAT_CRS = new Set([
  'urn:ogc:def:crs:ogc::crs84',
  'urn:ogc:def:crs:ogc:1.3:crs84',
  'crs84',
  'epsg:4326',
  'urn:ogc:def:crs:epsg::4326',
  'urn:ogc:def:crs:epsg:4326',
]);

export type Bounds = readonly [west: number, south: number, east: number, north: number];

/** Whether a stored `_crs` value names longitude/latitude degrees. None stated means RFC 7946's CRS84. */
export function isLonLatCrs(crs: unknown): boolean {
  if (crs === null || crs === undefined || crs === '') return true;
  let value: unknown = crs;
  if (typeof crs === 'string') {
    try {
      value = JSON.parse(crs);
    } catch {
      return LONLAT_CRS.has(crs.trim().toLowerCase());
    }
  }
  if (typeof value === 'string') return LONLAT_CRS.has(value.trim().toLowerCase());
  if (typeof value !== 'object' || value === null) return false;
  const named = (value as { properties?: { name?: unknown } }).properties?.name;
  if (typeof named === 'string') return LONLAT_CRS.has(named.trim().toLowerCase());
  const wkid = (value as { latestWkid?: unknown; wkid?: unknown }).latestWkid ?? (value as { wkid?: unknown }).wkid;
  return wkid === 4326;
}

function position(value: unknown): Position | null {
  if (!Array.isArray(value) || value.length < 2) return null;
  const [lon, lat] = value as unknown[];
  if (typeof lon !== 'number' || typeof lat !== 'number' || !Number.isFinite(lon) || !Number.isFinite(lat)) return null;
  // Degrees only: a projected coordinate (metres, feet) falls outside these ranges.
  if (lon < -180 || lon > 180 || lat < -90 || lat > 90) return null;
  return [lon, lat];
}

function ring(value: unknown): Ring | null {
  if (!Array.isArray(value)) return null;
  const points: Position[] = [];
  for (const item of value) {
    const point = position(item);
    if (!point) return null;
    points.push(point);
  }
  return points;
}

function polygon(value: unknown): PolygonCoordinates | null {
  if (!Array.isArray(value) || value.length === 0) return null;
  const rings: Ring[] = [];
  for (const item of value) {
    const read = ring(item);
    if (!read || read.length < 4) return null;
    rings.push(read);
  }
  return rings;
}

function countVertices(geometry: Geometry): number {
  switch (geometry.type) {
    case 'Point':
      return 1;
    case 'LineString':
      return geometry.coordinates.length;
    case 'Polygon':
      return geometry.coordinates.reduce((sum, item) => sum + item.length, 0);
    case 'MultiPolygon':
      return geometry.coordinates.reduce((sum, item) => sum + item.reduce((inner, part) => inner + part.length, 0), 0);
  }
}

/**
 * One stored feature row (`_geometry` JSON and `_crs`, as the tabular importer keeps them) as a
 * feature, or the reason it cannot be drawn.
 */
export function parseFeature(row: { _geometry: unknown; _crs?: unknown }): Feature | { unsupported: string } {
  if (!isLonLatCrs(row._crs)) return { unsupported: 'its coordinates are not longitude and latitude (CRS84 or EPSG:4326)' };
  let raw: unknown = row._geometry;
  if (typeof raw === 'string') {
    try {
      raw = JSON.parse(raw);
    } catch {
      return { unsupported: 'its geometry is not readable JSON' };
    }
  }
  if (typeof raw !== 'object' || raw === null) return { unsupported: 'it has no geometry' };
  const { type, coordinates } = raw as { type?: unknown; coordinates?: unknown };
  let geometry: Geometry | null;
  if (type === 'Point') {
    const point = position(coordinates);
    geometry = point && { type, coordinates: point };
  } else if (type === 'LineString') {
    const line = ring(coordinates);
    geometry = line && line.length >= 2 ? { type, coordinates: line } : null;
  } else if (type === 'Polygon') {
    const shape = polygon(coordinates);
    geometry = shape && { type, coordinates: shape };
  } else if (type === 'MultiPolygon' && Array.isArray(coordinates)) {
    const shapes = coordinates.map(polygon);
    geometry = shapes.length && shapes.every((shape) => shape !== null) ? { type, coordinates: shapes as PolygonCoordinates[] } : null;
  } else {
    return { unsupported: `geometry type ${String(type)} is not drawn` };
  }
  if (!geometry) return { unsupported: 'its coordinates are not longitude and latitude in degrees' };
  const vertices = countVertices(geometry);
  if (vertices > MAX_FEATURE_VERTICES) return { unsupported: `it has ${vertices.toLocaleString('en-US')} vertices; at most ${MAX_FEATURE_VERTICES.toLocaleString('en-US')} are drawn` };
  return { geometry, vertices };
}

function eachPosition(geometry: Geometry, visit: (point: Position) => void) {
  switch (geometry.type) {
    case 'Point':
      visit(geometry.coordinates);
      return;
    case 'LineString':
      geometry.coordinates.forEach(visit);
      return;
    case 'Polygon':
      geometry.coordinates.forEach((item) => item.forEach(visit));
      return;
    case 'MultiPolygon':
      geometry.coordinates.forEach((shape) => shape.forEach((item) => item.forEach(visit)));
  }
}

export function boundsOf(geometries: readonly Geometry[]): Bounds | null {
  let [west, south, east, north] = [Infinity, Infinity, -Infinity, -Infinity];
  for (const geometry of geometries) {
    eachPosition(geometry, ([lon, lat]) => {
      west = Math.min(west, lon);
      east = Math.max(east, lon);
      south = Math.min(south, lat);
      north = Math.max(north, lat);
    });
  }
  return Number.isFinite(west) ? [west, south, east, north] : null;
}

export type Projection = (lon: number, lat: number) => [number, number];

/**
 * Equirectangular projection of `bounds` into a box, longitude scaled by cos(the box's middle
 * latitude) so a city keeps its shape, centred with `padding` on every side.
 */
export function equirectangular(bounds: Bounds, { width, height, padding = 8 }: { width: number; height: number; padding?: number }): Projection {
  const [west, south, east, north] = bounds;
  const k = Math.cos(((south + north) / 2) * (Math.PI / 180));
  const spanX = Math.max(1e-9, (east - west) * k);
  const spanY = Math.max(1e-9, north - south);
  const scale = Math.min((width - 2 * padding) / spanX, (height - 2 * padding) / spanY);
  const offsetX = (width - spanX * scale) / 2;
  const offsetY = (height - spanY * scale) / 2;
  return (lon, lat) => [offsetX + (lon - west) * k * scale, offsetY + (north - lat) * scale];
}

/** A ring as path commands, dropping vertices within `tolerance` px of the last one kept. */
function ringPath(points: Ring, project: Projection, close: boolean, tolerance: number): string {
  let path = '';
  let last: [number, number] | null = null;
  points.forEach((point, index) => {
    const [x, y] = project(point[0], point[1]);
    const end = index === points.length - 1;
    if (last && !end && Math.abs(x - last[0]) < tolerance && Math.abs(y - last[1]) < tolerance) return;
    path += `${last ? 'L' : 'M'}${x.toFixed(1)},${y.toFixed(1)}`;
    last = [x, y];
  });
  return close && path ? `${path}Z` : path;
}

/** SVG path data for a polygon, multipolygon or line; a point has no path. */
export function geoPath(geometry: Geometry, project: Projection, tolerance = 0.5): string {
  switch (geometry.type) {
    case 'Point':
      return '';
    case 'LineString':
      return ringPath(geometry.coordinates, project, false, tolerance);
    case 'Polygon':
      return geometry.coordinates.map((item) => ringPath(item, project, true, tolerance)).join('');
    case 'MultiPolygon':
      return geometry.coordinates.map((shape) => shape.map((item) => ringPath(item, project, true, tolerance)).join('')).join('');
  }
}

/** Signed area and area-weighted centre of a closed ring, in degrees (planar). */
function ringMoments(points: Ring): { area: number; x: number; y: number } {
  let area = 0;
  let x = 0;
  let y = 0;
  for (let i = 0, j = points.length - 1; i < points.length; j = i, i += 1) {
    const [x0, y0] = points[j];
    const [x1, y1] = points[i];
    const cross = x0 * y1 - x1 * y0;
    area += cross;
    x += (x0 + x1) * cross;
    y += (y0 + y1) * cross;
  }
  return { area: area / 2, x, y };
}

/**
 * The area-weighted centroid of a polygon or multipolygon (holes subtract). Planar in degrees,
 * which at city scale lands within metres of the true centroid. Null for a shape with no area.
 */
export function centroid(geometry: Geometry): Position | null {
  const polygons = geometry.type === 'Polygon' ? [geometry.coordinates] : geometry.type === 'MultiPolygon' ? geometry.coordinates : null;
  if (!polygons) return geometry.type === 'Point' ? geometry.coordinates : null;
  let area = 0;
  let x = 0;
  let y = 0;
  for (const shape of polygons) {
    shape.forEach((item, index) => {
      const moments = ringMoments(item);
      // The outer ring adds and holes subtract, whatever winding the publisher used.
      const sign = index === 0 ? Math.sign(moments.area) : -Math.sign(moments.area);
      area += sign * moments.area;
      x += sign * moments.x;
      y += sign * moments.y;
    });
  }
  if (Math.abs(area) < 1e-15) return null;
  return [x / (6 * area), y / (6 * area)];
}

function inRing([px, py]: Position, points: Ring): boolean {
  let inside = false;
  for (let i = 0, j = points.length - 1; i < points.length; j = i, i += 1) {
    const [xi, yi] = points[i];
    const [xj, yj] = points[j];
    if ((yi > py) !== (yj > py) && px < ((xj - xi) * (py - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}

/** Whether a point lies inside a polygon or multipolygon, outside every hole (even-odd rule). */
export function pointInPolygon(point: Position, geometry: Geometry): boolean {
  const polygons = geometry.type === 'Polygon' ? [geometry.coordinates] : geometry.type === 'MultiPolygon' ? geometry.coordinates : [];
  return polygons.some(([outer, ...holes]) => inRing(point, outer) && !holes.some((hole) => inRing(point, hole)));
}
