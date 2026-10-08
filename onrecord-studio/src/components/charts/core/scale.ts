/**
 * Scales for the chart system. Everything temporal is UTC: stored values carry `Z`, the app has no
 * ICU time zones, and a date-only value means a calendar day, not an instant in the reader's zone.
 */

export interface ContinuousScale {
  (v: number): number;
  domain: readonly [number, number];
  range: readonly [number, number];
  invert(px: number): number;
  ticks(count?: number): number[];
}

export interface LinearOptions {
  /** Extend the domain to round tick values. Default true. */
  nice?: boolean;
  /**
   * Include zero in the domain. Default true: counts, sums and bars start at zero. A mean or rate
   * may opt out, and its caption must then say the axis does not start at zero.
   */
  zero?: boolean;
  clamp?: boolean;
}

/** The tick step nearest to span/count on the 1-2-5 ladder. */
export function tickStep(min: number, max: number, count: number): number {
  const raw = Math.abs(max - min) / Math.max(1, count);
  if (!Number.isFinite(raw) || raw === 0) return 1;
  const power = 10 ** Math.floor(Math.log10(raw));
  const error = raw / power;
  const factor = error >= 7.5 ? 10 : error >= 3.5 ? 5 : error >= 1.5 ? 2 : 1;
  return factor * power;
}

/** Round tick values covering [min, max]; the first and last ticks enclose the domain. */
export function niceTicks(min: number, max: number, count = 5): number[] {
  if (min === max) return [min];
  const [lo, hi] = min < max ? [min, max] : [max, min];
  const step = tickStep(lo, hi, count);
  const start = Math.ceil(lo / step - 1e-9);
  const stop = Math.floor(hi / step + 1e-9);
  const ticks: number[] = [];
  for (let i = start; i <= stop; i++) ticks.push(roundToStep(i * step, step));
  return ticks;
}

function roundToStep(value: number, step: number): number {
  const digits = Math.max(0, -Math.floor(Math.log10(step)) + 1);
  return Number(value.toFixed(Math.min(20, digits)));
}

function niceDomain(min: number, max: number, count: number): [number, number] {
  const step = tickStep(min, max, count);
  return [roundToStep(Math.floor(min / step) * step, step), roundToStep(Math.ceil(max / step) * step, step)];
}

function linearFrom(domain: readonly [number, number], range: readonly [number, number], clamp: boolean) {
  const [d0, d1] = domain;
  const [r0, r1] = range;
  const k = (r1 - r0) / (d1 - d0);
  const forward = (v: number) => {
    const px = r0 + (v - d0) * k;
    if (!clamp) return px;
    return Math.min(Math.max(px, Math.min(r0, r1)), Math.max(r0, r1));
  };
  const invert = (px: number) => d0 + (px - r0) / k;
  return { forward, invert };
}

export function scaleLinear(
  domain: readonly [number, number],
  range: readonly [number, number],
  { nice = true, zero = true, clamp = false }: LinearOptions = {},
): ContinuousScale {
  let lo = Math.min(domain[0], domain[1]);
  let hi = Math.max(domain[0], domain[1]);
  if (zero) {
    lo = Math.min(lo, 0);
    hi = Math.max(hi, 0);
  }
  if (lo === hi) {
    // A zero-span domain has no direction to scale in. A count axis of all zeros reads 0..1
    // (never a negative tick); anything else is centred on its one value.
    if (zero && lo === 0) hi = 1;
    else [lo, hi] = [lo - 1, hi + 1];
  }
  if (nice) [lo, hi] = niceDomain(lo, hi, 5);
  const resolved: readonly [number, number] = [lo, hi];
  const { forward, invert } = linearFrom(resolved, range, clamp);
  return Object.assign(forward, {
    domain: resolved,
    range,
    invert,
    ticks: (count = 5) => niceTicks(lo, hi, count),
  });
}

/** A log scale for strictly positive data. Zero or a negative bound has no logarithm. */
export function scaleLog(domain: readonly [number, number], range: readonly [number, number]): ContinuousScale {
  if (!(domain[0] > 0) || !(domain[1] > 0)) {
    throw new RangeError(`scaleLog needs a strictly positive domain, got [${domain[0]}, ${domain[1]}]`);
  }
  const [l0, l1] = [Math.log10(domain[0]), Math.log10(domain[1])];
  const span = l1 === l0 ? 1 : l1 - l0;
  const [r0, r1] = range;
  const forward = (v: number) => r0 + ((Math.log10(v) - l0) / span) * (r1 - r0);
  return Object.assign(forward, {
    domain,
    range,
    invert: (px: number) => 10 ** (l0 + ((px - r0) / (r1 - r0)) * span),
    ticks: () => {
      const ticks: number[] = [];
      for (let p = Math.ceil(Math.min(l0, l1)); p <= Math.floor(Math.max(l0, l1)); p++) ticks.push(10 ** p);
      return ticks;
    },
  });
}

/** Circle radius whose AREA is proportional to the value (radius ∝ √value). */
export function scaleSqrtArea(maxValue: number, maxRadius: number): (v: number) => number {
  return (v: number) => (maxValue <= 0 || v <= 0 ? 0 : Math.sqrt(Math.min(v, maxValue) / maxValue) * maxRadius);
}

export interface BandScale<K> {
  (k: K): number;
  bandwidth: number;
  step: number;
  domain: readonly K[];
}

export interface BandOptions {
  paddingInner?: number;
  paddingOuter?: number;
  /** The thickest a bar may be. Leftover band space is air, never a fatter bar. */
  maxBand?: number;
}

export function scaleBand<K>(
  domain: readonly K[],
  range: readonly [number, number],
  { paddingInner = 0.33, paddingOuter = 0.2, maxBand = 24 }: BandOptions = {},
): BandScale<K> {
  const [r0, r1] = range;
  const n = domain.length;
  const step = n === 0 ? 0 : (r1 - r0) / Math.max(1, n - paddingInner + 2 * paddingOuter);
  const natural = step * (1 - paddingInner);
  const bandwidth = Math.max(0, Math.min(natural, maxBand));
  const index = new Map(domain.map((k, i) => [k, i] as const));
  const position = (k: K) => {
    const i = index.get(k);
    if (i === undefined) return Number.NaN;
    return r0 + step * paddingOuter + i * step + (natural - bandwidth) / 2;
  };
  return Object.assign(position, { bandwidth, step, domain });
}

// ---- Time ----------------------------------------------------------------------------------

export type TimeUnit = 'year' | 'fiscal-year' | 'quarter' | 'month' | 'week' | 'day' | 'hour';

export interface TimeTick {
  at: number;
  unit: TimeUnit;
  /** The tick also starts the next-larger unit (a January month tick starts a year). */
  major: boolean;
}

export interface TimeScale {
  (ms: number): number;
  domain: readonly [number, number];
  range: readonly [number, number];
  invert(px: number): number;
  ticks(target?: number): TimeTick[];
  /** First month of the fiscal year (1-12), when the axis is fiscal. */
  fiscalYearStartMonth?: number;
}

const HOUR = 3_600_000;
const DAY = 24 * HOUR;

/** Parses an ISO date or timestamp. A date-only value is that calendar day at UTC midnight. */
export function parseDate(value: string): number {
  const dateOnly = /^(\d{4})-(\d{2})-(\d{2})$/.exec(value);
  if (dateOnly) return Date.UTC(Number(dateOnly[1]), Number(dateOnly[2]) - 1, Number(dateOnly[3]));
  return Date.parse(value);
}

function monthStart(year: number, month: number): number {
  return Date.UTC(year, month, 1);
}

/** Calendar ticks of one unit and multiple between two instants, UTC. */
function calendarTicks(lo: number, hi: number, unit: TimeUnit, every: number, fyStart: number): TimeTick[] {
  const ticks: TimeTick[] = [];
  const start = new Date(lo);
  const push = (at: number, major: boolean) => {
    if (at >= lo && at <= hi) ticks.push({ at, unit, major });
  };
  if (unit === 'hour' || unit === 'day' || unit === 'week') {
    const size = unit === 'hour' ? HOUR : DAY;
    const base = unit === 'week'
      // Weeks start on Monday.
      ? Date.UTC(start.getUTCFullYear(), start.getUTCMonth(), start.getUTCDate() - ((start.getUTCDay() + 6) % 7))
      : unit === 'day'
        ? Date.UTC(start.getUTCFullYear(), start.getUTCMonth(), start.getUTCDate())
        : Math.floor(lo / HOUR) * HOUR;
    const stride = (unit === 'week' ? 7 : 1) * every * size;
    const aligned = unit === 'week' ? base : Math.floor(base / (every * size)) * every * size;
    for (let at = aligned; at <= hi; at += stride) {
      const d = new Date(at);
      const major = unit === 'hour' ? d.getUTCHours() === 0 : d.getUTCDate() <= (unit === 'week' ? 7 : 1);
      push(at, major);
    }
    return ticks;
  }
  // Month-based units count months from year 0, so every grid (quarters, half-years, fiscal
  // years starting in July, every 5th year) is one modulus.
  const months = unit === 'quarter' ? 3 * every : unit === 'month' ? every : 12 * every;
  const offset = unit === 'fiscal-year' ? fyStart - 1 : 0;
  const first = start.getUTCFullYear() * 12 + start.getUTCMonth();
  const last = new Date(hi).getUTCFullYear() * 12 + new Date(hi).getUTCMonth();
  for (let index = Math.ceil((first - offset) / months) * months + offset; index <= last; index += months) {
    const month = index % 12;
    const major = unit === 'month' || unit === 'quarter' ? month === fyStart - 1 : true;
    push(monthStart(Math.floor(index / 12), month), major);
  }
  return ticks;
}

const LADDER: readonly { unit: TimeUnit; every: number; approx: number }[] = [
  { unit: 'hour', every: 1, approx: HOUR },
  { unit: 'hour', every: 3, approx: 3 * HOUR },
  { unit: 'hour', every: 6, approx: 6 * HOUR },
  { unit: 'hour', every: 12, approx: 12 * HOUR },
  { unit: 'day', every: 1, approx: DAY },
  { unit: 'day', every: 2, approx: 2 * DAY },
  { unit: 'week', every: 1, approx: 7 * DAY },
  { unit: 'month', every: 1, approx: 30.4 * DAY },
  { unit: 'quarter', every: 1, approx: 91.3 * DAY },
  { unit: 'month', every: 6, approx: 182.6 * DAY },
  { unit: 'year', every: 1, approx: 365.25 * DAY },
  { unit: 'year', every: 2, approx: 730.5 * DAY },
  { unit: 'year', every: 5, approx: 1826 * DAY },
  { unit: 'year', every: 10, approx: 3652 * DAY },
];

export interface TimeOptions {
  /** 7 = a July-to-June fiscal year (City of San Diego). Year ticks become fiscal-year ticks. */
  fiscalYearStartMonth?: number;
}

export function scaleTime(
  domainMs: readonly [number, number],
  range: readonly [number, number],
  { fiscalYearStartMonth }: TimeOptions = {},
): TimeScale {
  let [lo, hi] = domainMs[0] <= domainMs[1] ? domainMs : [domainMs[1], domainMs[0]];
  // One instant has no span to tick across: it gets a single tick at itself, as a fiscal year or a
  // day, never a ladder of hours around it.
  const instant = lo === hi ? lo : null;
  if (lo === hi) [lo, hi] = [lo - DAY / 2, hi + DAY / 2];
  const resolved: readonly [number, number] = [lo, hi];
  const { forward, invert } = linearFrom(resolved, range, false);
  const fyStart = fiscalYearStartMonth ?? 1;
  return Object.assign(forward, {
    domain: resolved,
    range,
    invert,
    fiscalYearStartMonth,
    ticks: (target = 6): TimeTick[] => {
      if (instant !== null) return [{ at: instant, unit: fiscalYearStartMonth && fiscalYearStartMonth !== 1 ? 'fiscal-year' : 'day', major: true }];
      const span = hi - lo;
      const choice = LADDER.find((rung) => span / rung.approx <= target) ?? LADDER[LADDER.length - 1];
      const unit: TimeUnit = choice.unit === 'year' && fiscalYearStartMonth && fiscalYearStartMonth !== 1 ? 'fiscal-year' : choice.unit;
      return calendarTicks(lo, hi, unit, choice.every, fyStart);
    },
  });
}
