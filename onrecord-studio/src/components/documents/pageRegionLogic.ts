import type { PageRectangle } from '../../types';

export interface PagePoint {
  x: number;
  y: number;
}

export interface PageBox {
  width: number;
  height: number;
}

export type NormalisedRectangle = Omit<PageRectangle, 'pageIndex'>;

/** Coordinates are kept to a millionth of the page; Rust accepts a rounding excess of a billionth. */
const PRECISION = 1_000_000;

const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));
const quantise = (value: number) => Math.round(value * PRECISION);

/**
 * The rectangle between two points on a page box, as fractions of the page with the origin at the
 * top left. Both points are in the same units as `box` (any zoom), so the result does not depend on
 * the zoom the page is shown at. Points outside the page are pulled onto its edge. Null when the
 * rectangle has no area at this precision.
 */
export function normaliseRectangle(from: PagePoint, to: PagePoint, box: PageBox): NormalisedRectangle | null {
  if (!(box.width > 0) || !(box.height > 0)) return null;
  const edges = [from.x, from.y, to.x, to.y];
  if (!edges.every(Number.isFinite)) return null;
  const left = quantise(clamp(Math.min(from.x, to.x) / box.width, 0, 1));
  const right = quantise(clamp(Math.max(from.x, to.x) / box.width, 0, 1));
  const top = quantise(clamp(Math.min(from.y, to.y) / box.height, 0, 1));
  const bottom = quantise(clamp(Math.max(from.y, to.y) / box.height, 0, 1));
  if (right <= left || bottom <= top) return null;
  return { x: left / PRECISION, y: top / PRECISION, width: (right - left) / PRECISION, height: (bottom - top) / PRECISION };
}

export interface ClientBox {
  left: number;
  top: number;
  right: number;
  bottom: number;
}

/** The smallest normalised rectangle containing every client box, against the page's own client box. */
export function normaliseClientBoxes(boxes: readonly ClientBox[], page: ClientBox): NormalisedRectangle | null {
  const usable = boxes.filter((box) => box.right > box.left && box.bottom > box.top);
  if (usable.length === 0) return null;
  return normaliseRectangle(
    { x: Math.min(...usable.map((box) => box.left)) - page.left, y: Math.min(...usable.map((box) => box.top)) - page.top },
    { x: Math.max(...usable.map((box) => box.right)) - page.left, y: Math.max(...usable.map((box) => box.bottom)) - page.top },
    { width: page.right - page.left, height: page.bottom - page.top },
  );
}

export interface OriginalHeader {
  byteLength: number;
  sha256: string;
  recordedSha256: string | null;
  matchesRecorded: boolean | null;
}

/** Splits what `read_document_original` returns: the file, then a JSON header, then its length as a big-endian u32. */
export function parseOriginalFrame(frame: ArrayBuffer): { bytes: Uint8Array; header: OriginalHeader } {
  if (frame.byteLength < 4) throw new Error('The original file response was incomplete.');
  const length = new DataView(frame, frame.byteLength - 4, 4).getUint32(0, false);
  const headerEnd = frame.byteLength - 4;
  if (length > headerEnd) throw new Error('The original file response was incomplete.');
  const header = JSON.parse(new TextDecoder().decode(new Uint8Array(frame, headerEnd - length, length))) as OriginalHeader;
  if (header.byteLength !== headerEnd - length) throw new Error('The original file response does not match its header.');
  return { bytes: new Uint8Array(frame, 0, header.byteLength), header };
}

/** Text layer text of a region: whitespace collapsed, control characters dropped, nothing truncated. */
export function cleanRegionText(text: string): string {
  // eslint-disable-next-line no-control-regex
  return text.replace(/[\u0000-\u001f\u007f]+/g, ' ').replace(/\s+/g, ' ').trim();
}

export function percent(value: number): string {
  return `${(value * 100).toFixed(1)}%`;
}
