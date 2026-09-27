import type { ServiceObservation } from '../types/monitoring';

export function observationPoints(observations: ServiceObservation[]) {
  const located = observations.filter((o): o is ServiceObservation & { latitude: number; longitude: number } =>
    o.latitude !== null && o.longitude !== null && Number.isFinite(o.latitude) && Number.isFinite(o.longitude));
  if (!located.length) return [];
  const latitudes = located.map(o => o.latitude); const longitudes = located.map(o => o.longitude);
  const minLat = Math.min(...latitudes); const maxLat = Math.max(...latitudes);
  const minLon = Math.min(...longitudes); const maxLon = Math.max(...longitudes);
  return located.map(observation => ({ observation,
    x: maxLon === minLon ? 300 : 30 + (observation.longitude - minLon) / (maxLon - minLon) * 540,
    y: maxLat === minLat ? 150 : 270 - (observation.latitude - minLat) / (maxLat - minLat) * 240,
  }));
}

export function optionalMeasurement(value: string): number | null {
  if (!value.trim()) return null;
  const number = Number(value);
  if (!Number.isFinite(number)) throw new Error('Use a finite number or leave the unknown measurement blank.');
  return number;
}

export function citationLocation(source: { snapshotId: string | null; rowNumber: number | null; characterOffset: number }) {
  return source.snapshotId ? `Table ${source.snapshotId}, row ${source.rowNumber}` : `Retained text, character ${source.characterOffset}`;
}
