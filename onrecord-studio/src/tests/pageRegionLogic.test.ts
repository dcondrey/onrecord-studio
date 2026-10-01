import { describe, expect, it } from 'vitest';
import { annotationCitation, REGION_STATEMENT } from '../components/documents/annotationLogic';
import { normaliseClientBoxes, normaliseRectangle, parseOriginalFrame } from '../components/documents/pageRegionLogic';
import type { ResolvedAnnotation } from '../types';

describe('normaliseRectangle', () => {
  it('gives the same rectangle for the same area of the page at any zoom, whichever corner the drag started from', () => {
    const at = (zoom: number) => normaliseRectangle({ x: 750 * zoom, y: 600 * zoom }, { x: 250 * zoom, y: 200 * zoom }, { width: 1000 * zoom, height: 800 * zoom });
    for (const zoom of [0.5, 1, 1.25, 3]) expect(at(zoom)).toEqual({ x: 0.25, y: 0.25, width: 0.5, height: 0.5 });
  });

  it('pulls points onto the page and never produces a rectangle Rust would refuse', () => {
    const box = { width: 300, height: 400 };
    const whole = normaliseRectangle({ x: -50, y: -50 }, { x: 900, y: 900 }, box);
    expect(whole).toEqual({ x: 0, y: 0, width: 1, height: 1 });
    for (let start = 0; start < 300; start += 7) {
      const found = normaliseRectangle({ x: start + 0.123, y: 3.3 }, { x: 299.999, y: 399.5 }, box);
      if (found) expect(found.x + found.width).toBeLessThanOrEqual(1 + 1e-9);
    }
  });

  it('refuses a rectangle with no area, a page with no size, and non-finite points', () => {
    const box = { width: 300, height: 400 };
    expect(normaliseRectangle({ x: 10, y: 10 }, { x: 10, y: 90 }, box)).toBeNull();
    expect(normaliseRectangle({ x: 10, y: 10 }, { x: 90, y: 10 }, box)).toBeNull();
    expect(normaliseRectangle({ x: -20, y: 10 }, { x: -5, y: 90 }, box)).toBeNull();
    expect(normaliseRectangle({ x: 1, y: 1 }, { x: 9, y: 9 }, { width: 0, height: 400 })).toBeNull();
    expect(normaliseRectangle({ x: Number.NaN, y: 1 }, { x: 9, y: 9 }, box)).toBeNull();
  });

  it('boxes a selection from its client rectangles against the page', () => {
    const page = { left: 100, top: 50, right: 400, bottom: 450 };
    const found = normaliseClientBoxes([{ left: 130, top: 90, right: 190, bottom: 110 }, { left: 130, top: 110, right: 250, bottom: 130 }], page);
    expect(found).toEqual({ x: 0.1, y: 0.1, width: 0.4, height: 0.1 });
    expect(normaliseClientBoxes([{ left: 1, top: 1, right: 1, bottom: 5 }], page)).toBeNull();
  });
});

describe('parseOriginalFrame', () => {
  it('splits the file from its trailing header and refuses a truncated frame', () => {
    const file = new TextEncoder().encode('%PDF-1.7');
    const header = new TextEncoder().encode(JSON.stringify({ byteLength: file.length, sha256: 'a'.repeat(64), recordedSha256: null, matchesRecorded: null }));
    const frame = new Uint8Array(file.length + header.length + 4);
    frame.set(file, 0);
    frame.set(header, file.length);
    new DataView(frame.buffer).setUint32(file.length + header.length, header.length, false);
    const parsed = parseOriginalFrame(frame.buffer);
    expect(new TextDecoder().decode(parsed.bytes)).toBe('%PDF-1.7');
    expect(parsed.header.matchesRecorded).toBeNull();
    expect(() => parseOriginalFrame(frame.buffer.slice(0, frame.length - 6))).toThrow();
  });
});

describe('a region citation', () => {
  const region: ResolvedAnnotation = {
    id: 'r1',
    portalKey: 'sd',
    documentId: 7,
    kind: 'ISSUE',
    characterOffset: null,
    quote: '',
    prefix: '',
    suffix: '',
    textSha256: null,
    contextUnanchored: false,
    note: '',
    tag: 'AMEND',
    author: 'Maria',
    status: 'OPEN',
    findingId: null,
    importedFromKey: null,
    createdAt: '2026-01-01T00:00:00Z',
    pageRegion: { pageIndex: 2, x: 0.1, y: 0.2, width: 0.3, height: 0.4, originalSha256: 'cd'.repeat(32) },
    anchor: null,
  };

  it('states the page, rectangle, original digest and that it locates the original, not the text', () => {
    const text = annotationCitation(region, { fileName: 'a.pdf', currentTextSha256: null, appVersion: '1.0.0', regionState: 'ANCHORED' });
    expect(text).toContain('Page: 3 (page index 2');
    expect(text).toContain('left 10.0%, top 20.0%, width 30.0%, height 40.0%');
    expect(text).toContain(`Original file SHA-256: ${'cd'.repeat(32)}`);
    expect(text).toContain('none (an image or a scan');
    expect(text).toContain(REGION_STATEMENT);
    expect(text).not.toContain('Character offset');
  });

  it('carries the text found in the rectangle and the warning when the original changed', () => {
    const text = annotationCitation({ ...region, quote: 'fee waived' }, { currentTextSha256: null, appVersion: '1.0.0', regionState: 'ORIGINAL_CHANGED' });
    expect(text).toContain('"fee waived"');
    expect(text).toContain('The original file has changed since this region was drawn');
  });
});
