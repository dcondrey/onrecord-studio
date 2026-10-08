import { readFileSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';
import { scaleBand, scaleLinear, scaleLog, scaleTime, parseDate } from '../components/charts/core/scale';
import { formatCurrencyString, formatDate, formatFiscalYear, formatValue } from '../components/charts/core/format';
import { fromNullable, segmentsOf, type Measured } from '../components/charts/core/nulls';
import { assertYardstickLabel, paintFor, VIZ, type MarkRole, type MarkVariant } from '../components/charts/core/claim';
import { beeswarm, placeLabels } from '../components/charts/core/layout';

describe('chart tokens', () => {
  // The contrast-more light block outranks the plain dark block, so any chart role it sets must be
  // restated for dark + more contrast or dark readers who ask for more contrast get light marks.
  const css = readFileSync(join(__dirname, '..', 'index.css'), 'utf8');
  const names = (selector: string) => {
    const start = css.indexOf(`${selector} {`);
    return new Set([...css.slice(start, css.indexOf('}', start)).matchAll(/--(viz-[\w-]+):/g)].map(([, name]) => name));
  };
  it('define every chart role for light and dark, and restate every contrast-more role for dark', () => {
    const light = names(':root');
    const dark = names(':root[data-theme="dark"]');
    const roles = ['surface', 'ink', 'mark', 'context', 'track', 'grid', 'baseline', 'lead', 'positive', 'counted', 'dim',
      'ord-1', 'ord-2', 'ord-3', 'ord-4', 'seq-1', 'seq-2', 'seq-3', 'seq-4', 'seq-5'].map((role) => `viz-${role}`);
    expect(roles.filter((role) => !light.has(role))).toEqual([]);
    const accentsAndSteps = roles.filter((role) => /ink|mark|context|track|lead|positive|counted|ord|seq/.test(role));
    expect(accentsAndSteps.filter((role) => !dark.has(role))).toEqual([]);
    const moreDark = names(':root[data-contrast="more"][data-theme="dark"]');
    expect([...names(':root[data-contrast="more"]')].filter((role) => !moreDark.has(role))).toEqual([]);
  });
});

describe('scales', () => {
  it('cap a band at 24px: 23 stays 23, 24 stays 24, 25 becomes 24', () => {
    // One band with the default paddings: bandwidth = span * 0.67 / (1 - 0.33 + 0.4).
    const spanFor = (band: number) => (band / 0.67) * 1.07;
    expect(scaleBand(['a'], [0, spanFor(23)]).bandwidth).toBeCloseTo(23, 6);
    expect(scaleBand(['a'], [0, spanFor(24)]).bandwidth).toBeCloseTo(24, 6);
    expect(scaleBand(['a'], [0, spanFor(25)]).bandwidth).toBe(24);
  });

  it('give a zero-span domain a direction, and an all-zero count axis no negative tick', () => {
    const counts = scaleLinear([0, 0], [0, 100]);
    expect(counts.domain).toEqual([0, 1]);
    expect(Math.min(...counts.ticks())).toBe(0);
    const mean = scaleLinear([41.2, 41.2], [0, 100], { zero: false, nice: false });
    expect(mean.domain).toEqual([40.2, 42.2]);
    expect(Number.isFinite(mean(41.2))).toBe(true);
  });

  it('refuse a log domain that is not strictly positive', () => {
    expect(() => scaleLog([0, 100], [0, 1])).toThrow(RangeError);
    expect(() => scaleLog([-1, 100], [0, 1])).toThrow(RangeError);
  });

  it('put fiscal-year ticks on the fiscal year start', () => {
    const scale = scaleTime([parseDate('2019-07-01'), parseDate('2025-06-30')], [0, 600], { fiscalYearStartMonth: 7 });
    const ticks = scale.ticks(8);
    expect(ticks.length).toBeGreaterThan(0);
    expect(ticks.every((tick) => tick.unit === 'fiscal-year' && new Date(tick.at).getUTCMonth() === 6 && new Date(tick.at).getUTCDate() === 1)).toBe(true);
  });
});

describe('formatters', () => {
  it("state a rate difference in percentage points, never a relative change", () => {
    expect(formatValue(0.082 - 0.08, { kind: 'pts' })).toBe('+0.2 pts');
    expect(formatValue(0.08 - 0.082, { kind: 'pts' })).toBe('−0.2 pts');
  });

  it('keep a date-only value on its calendar day in a negative-offset zone', () => {
    const previous = process.env.TZ;
    process.env.TZ = 'America/Los_Angeles';
    try {
      expect(new Date(2024, 0, 1).getTimezoneOffset()).toBeGreaterThan(0);
      expect(formatDate(parseDate('2024-03-01'), 'day')).toBe('Mar 1, 2024');
      expect(formatFiscalYear(parseDate('2024-06-30')).label).toBe('FY24');
      expect(formatFiscalYear(parseDate('2024-07-01'))).toEqual({ label: 'FY25', range: 'Jul 2024 to Jun 2025' });
    } finally {
      process.env.TZ = previous;
    }
  });

  it('print a compact amount from the decimal string, not a float', () => {
    // 9007199254740993 is not representable as a double; Number() would print ...992.
    expect(formatCurrencyString('9007199254740993.50')).toBe('$9,007,199,254,740,993.50');
    expect(formatCurrencyString('4213450.75', { compact: true })).toBe('$4.2M');
    expect(formatCurrencyString('999950', { compact: true })).toBe('$1M');
    expect(formatCurrencyString('-236200', { signed: true })).toBe('−$236,200.00');
  });
});

describe('gap model', () => {
  it('never turns a null into the value 0', () => {
    const m: Measured = fromNullable<number>(null, 'not-recorded', 'no closures measured');
    expect(m.state).toBe('not-recorded');
    expect('value' in m).toBe(false);
    expect(fromNullable(0, 'not-recorded', 'unused')).toEqual({ state: 'value', value: 0 });
  });

  it('never bridges an absent point', () => {
    const points = [1, null, 2, 3, null, null, 4].map((y, x) => ({ x, y }));
    const runs = segmentsOf(points, (p) => p.y !== null);
    expect(runs.map((run) => run.map((p) => p.x))).toEqual([[0], [2, 3], [6]]);
    for (const run of runs) {
      for (let i = 1; i < run.length; i++) expect(run[i].x - run[i - 1].x).toBe(1);
    }
  });
});

describe('claim paint', () => {
  it('never resolves the counted colour for a statistical or descriptive claim', () => {
    const variants: MarkVariant[] = ['fill', 'outline', 'hollow', 'line'];
    const roles: MarkRole[] = ['primary', 'default', 'comparison', 'lead', 'positive'];
    for (const claim of ['statistical', 'descriptive'] as const) {
      for (const variant of variants) {
        for (const role of roles) {
          const paint = paintFor(claim, variant, role);
          expect([paint.fill, paint.stroke]).not.toContain(VIZ.counted);
          if (claim === 'statistical') expect(paint.fill === 'none' || paint.fill === VIZ.surface).toBe(true);
        }
      }
    }
    expect(paintFor('counted', 'fill').fill).toBe(VIZ.counted);
  });

  it('refuse a yardstick label that does not say it is a reference', () => {
    expect(() => assertYardstickLabel('72 h before start')).toThrow();
    expect(() => assertYardstickLabel('72 h before start (reference; no notice rule is evaluated)')).not.toThrow();
    expect(() => assertYardstickLabel('Lead threshold (10% and $1M): app yardstick, not a budget rule')).not.toThrow();
  });
});

describe('layout', () => {
  it('lays out a beeswarm identically whatever the input order, with no overlaps', () => {
    const items = [5, 5, 5, 5.2, 9, 1, 5.1].map((value, i) => ({ key: `k${i}`, value }));
    const x = scaleLinear([0, 10], [0, 200]);
    const a = beeswarm(items, x, 4, 4);
    const b = beeswarm([...items].reverse(), x, 4, 4);
    expect(b).toEqual(a);
    for (let i = 0; i < a.length; i++) {
      for (let j = i + 1; j < a.length; j++) expect(Math.hypot(a[i].cx - a[j].cx, a[i].dy - a[j].dy)).toBeGreaterThanOrEqual(9);
    }
  });

  it('places labels without overlap inside the bounds', () => {
    const labels = [{ key: 'a', anchor: 10, width: 40 }, { key: 'b', anchor: 12, width: 40 }, { key: 'c', anchor: 195, width: 30 }];
    const placed = placeLabels(labels, 0, 200);
    const spans = placed.map((p, i) => [p.pos, p.pos + labels[i].width]).sort((m, n) => m[0] - n[0]);
    for (let i = 1; i < spans.length; i++) expect(spans[i][0]).toBeGreaterThanOrEqual(spans[i - 1][1] + 4);
    expect(spans[0][0]).toBeGreaterThanOrEqual(0);
    expect(spans[spans.length - 1][1]).toBeLessThanOrEqual(200);
  });
});
