import { describe, expect, it } from 'vitest';
import { centroid, equirectangular, MAX_FEATURE_VERTICES, parseFeature, pointInPolygon, type Geometry } from '../components/charts/core/geo';

const square = (west: number, south: number, size: number) => [[west, south], [west + size, south], [west + size, south + size], [west, south + size], [west, south]];

describe('place-map geography', () => {
  it('reads only longitude and latitude, refusing a projected file rather than drawing it somewhere wrong', () => {
    const geometry = JSON.stringify({ type: 'Polygon', coordinates: [square(-117.2, 32.7, 0.01)] });
    expect('geometry' in parseFeature({ _geometry: geometry, _crs: '{"type":"name","properties":{"name":"urn:ogc:def:crs:OGC::CRS84"}}' })).toBe(true);
    // No crs member is RFC 7946's CRS84.
    expect('geometry' in parseFeature({ _geometry: geometry })).toBe(true);
    expect(parseFeature({ _geometry: geometry, _crs: '{"type":"name","properties":{"name":"urn:ogc:def:crs:EPSG::2230"}}' })).toEqual({ unsupported: 'its coordinates are not longitude and latitude (CRS84 or EPSG:4326)' });
    // State-plane feet with no crs stated fall outside degree ranges.
    const feet = JSON.stringify({ type: 'Polygon', coordinates: [square(6_280_000, 1_840_000, 500)] });
    expect('unsupported' in parseFeature({ _geometry: feet })).toBe(true);
  });

  it('draws a feature of 200,000 vertices and refuses one of 200,001', () => {
    const ring = (vertices: number) => {
      const points = Array.from({ length: vertices - 1 }, (_, i) => [-117 + Math.cos(i) * 0.01, 32.7 + Math.sin(i) * 0.01]);
      return [...points, points[0]];
    };
    const feature = (vertices: number) => parseFeature({ _geometry: { type: 'Polygon', coordinates: [ring(vertices)] } });
    expect('geometry' in feature(MAX_FEATURE_VERTICES - 1)).toBe(true);
    expect('geometry' in feature(MAX_FEATURE_VERTICES)).toBe(true);
    expect('unsupported' in feature(MAX_FEATURE_VERTICES + 1)).toBe(true);
  });

  it('places a centroid and a point test with holes subtracted, whatever the ring winding', () => {
    const outer = square(0, 0, 4);
    const hole = square(0, 0, 2).reverse();
    const shape: Geometry = { type: 'Polygon', coordinates: [outer.reverse() as [number, number][], hole as [number, number][]] };
    // 16 - 4 = 12 units of area; the hole pulls the centre away from it.
    const [x, y] = centroid(shape)!;
    expect(x).toBeCloseTo((16 * 2 - 4 * 1) / 12);
    expect(y).toBeCloseTo((16 * 2 - 4 * 1) / 12);
    expect(pointInPolygon([1, 1], shape)).toBe(false);
    expect(pointInPolygon([3, 3], shape)).toBe(true);
    expect(pointInPolygon([5, 5], shape)).toBe(false);
  });

  it('keeps a city its shape: a degree of longitude is cos(latitude) of a degree of latitude', () => {
    const project = equirectangular([-117.3, 32.5, -116.9, 33.1], { width: 400, height: 400, padding: 0 });
    const [x0, y0] = project(-117.1, 32.8);
    const [x1] = project(-117.0, 32.8);
    const [, y1] = project(-117.1, 32.9);
    expect((x1 - x0) / (y0 - y1)).toBeCloseTo(Math.cos((32.8 * Math.PI) / 180), 3);
  });
});
