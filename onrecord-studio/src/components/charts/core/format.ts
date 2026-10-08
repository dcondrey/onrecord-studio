import { financialAmount } from '../../../utils/financial';
import type { TimeUnit } from './scale';

export type ValueKind = 'count' | 'currency' | 'percent' | 'pts' | 'days' | 'hours' | 'bytes' | 'basisPoints' | 'z';

export interface FormatOptions {
  kind: ValueKind;
  /** ISO 4217 code for 'currency'. Default USD. */
  currency?: string;
  /** 1,284 / 12.9K / $4.2M. */
  compact?: boolean;
  /** Prefix '+' on positive values. 'pts' is a difference and is always signed. */
  signed?: boolean;
  /** An estimate from a sample (ESTIMATED_FROM_SAMPLE): prefixed '≈'. */
  approximate?: boolean;
  /** Fraction digits; each kind has its own default. */
  digits?: number;
}

const MINUS = '−';

function grouped(v: number, digits: number): string {
  return v.toLocaleString('en-US', { minimumFractionDigits: digits, maximumFractionDigits: digits });
}

function compactNumber(v: number): string {
  const abs = Math.abs(v);
  const tiers: [number, string][] = [[1e12, 'T'], [1e9, 'B'], [1e6, 'M'], [1e3, 'K']];
  for (const [size, suffix] of tiers) {
    if (abs >= size * 0.9995) {
      const scaled = abs / size;
      return `${scaled >= 100 ? Math.round(scaled) : scaled.toFixed(1).replace(/\.0$/, '')}${suffix}`;
    }
  }
  return grouped(abs, abs % 1 === 0 ? 0 : 1);
}

function currencySymbol(code: string): string {
  return code === 'USD' ? '$' : `${code} `;
}

/** Applies the sign (a true minus) and the '≈' estimate mark. A value that rounds to zero is unsigned. */
function signedText(v: number, body: string, signed: boolean, approximate: boolean): string {
  const nonzero = /[1-9]/.test(body);
  const sign = v < 0 && nonzero ? MINUS : signed && v > 0 && nonzero ? '+' : '';
  return `${approximate ? '≈' : ''}${sign}${body}`;
}

export function formatValue(v: number, { kind, currency = 'USD', compact = false, signed = false, approximate = false, digits }: FormatOptions): string {
  const abs = Math.abs(v);
  switch (kind) {
    case 'count':
      return signedText(v, compact ? compactNumber(abs) : grouped(abs, digits ?? 0), signed, approximate);
    case 'currency': {
      const body = compact ? compactNumber(abs) : grouped(abs, digits ?? (abs % 1 === 0 ? 0 : 2));
      return signedText(v, `${currencySymbol(currency)}${body}`, signed, approximate);
    }
    case 'percent':
      return signedText(v, `${grouped(abs * 100, digits ?? 1)}%`, signed, approximate);
    case 'pts':
      // A difference between two rates, in percentage points. A relative change of a rate is not
      // expressible here on purpose: (0.082 − 0.08) / 0.08 overstates a 0.2-point move as 2.5%.
      return signedText(v, `${grouped(abs * 100, digits ?? 1)} pts`, true, approximate);
    case 'days':
      return signedText(v, `${grouped(abs, digits ?? (abs >= 100 ? 0 : 1))} d`, signed, approximate);
    case 'hours':
      return signedText(v, `${grouped(abs, digits ?? 0)} h`, signed, approximate);
    case 'bytes': {
      const units = ['B', 'KB', 'MB', 'GB', 'TB'];
      let scaled = abs;
      let unit = 0;
      while (scaled >= 1024 && unit < units.length - 1) {
        scaled /= 1024;
        unit += 1;
      }
      return signedText(v, `${grouped(scaled, digits ?? (unit === 0 ? 0 : 1))} ${units[unit]}`, signed, approximate);
    }
    case 'basisPoints':
      return signedText(v, `${grouped(abs, digits ?? 0)} bp`, signed, approximate);
    case 'z':
      return `z ${signedText(v, grouped(abs, digits ?? 1), true, approximate)}`;
  }
}

/** A tick label whose precision follows the tick step, so 0.5-step ticks never print as '1'. */
export function formatTick(v: number, kind: ValueKind, step: number): string {
  const scale = kind === 'percent' ? 100 : 1;
  const digits = Math.max(0, Math.min(6, -Math.floor(Math.log10(Math.abs(step * scale) || 1) + 1e-9)));
  if (kind === 'pts') return formatValue(v, { kind, digits });
  if (kind === 'currency' || (kind === 'count' && Math.abs(v) >= 10_000)) return formatValue(v, { kind, compact: Math.abs(v) >= 10_000, digits });
  return formatValue(v, { kind, digits });
}

/**
 * A decimal amount string for display. The digits come from the string (BigInt), never from a
 * float: geometry may use Number(), a printed amount may not.
 */
export function formatCurrencyString(value: string, { currency = 'USD', compact = false, signed = false }: { currency?: string; compact?: boolean; signed?: boolean } = {}): string {
  const match = /^(-?)(\d+)(?:\.(\d+))?$/.exec(value.trim());
  if (!match) return value;
  const negative = match[1] === '-' && /[1-9]/.test(match[2] + (match[3] ?? ''));
  const sign = negative ? MINUS : signed && /[1-9]/.test(match[2] + (match[3] ?? '')) ? '+' : '';
  const symbol = currencySymbol(currency);
  if (!compact) return `${sign}${symbol}${financialAmount(`${match[2]}.${match[3] ?? ''}`.replace(/\.$/, ''))}`;
  const whole = BigInt(match[2]);
  const digitsLong = whole.toString().length;
  let tier = Math.min(4, Math.floor((digitsLong - 1) / 3));
  if (tier === 0) return `${sign}${symbol}${whole.toString()}`;
  const suffixes = ['', 'K', 'M', 'B', 'T'];
  // Tenths of the tier unit, rounded half up; 999,950 rounds to 1.0M, not 1,000.0K.
  let tenths = (whole + 10n ** BigInt(tier * 3 - 1) / 2n) / 10n ** BigInt(tier * 3 - 1);
  if (tenths >= 10_000n && tier < 4) {
    tier += 1;
    tenths = (whole + 10n ** BigInt(tier * 3 - 1) / 2n) / 10n ** BigInt(tier * 3 - 1);
  }
  const integer = tenths / 10n;
  const fraction = tenths % 10n;
  const body = integer >= 100n || fraction === 0n ? `${integer}` : `${integer}.${fraction}`;
  return `${sign}${symbol}${body}${suffixes[tier]}`;
}

// ---- Dates (UTC throughout) -----------------------------------------------------------------

const FORMATS: Record<Exclude<TimeUnit, 'fiscal-year' | 'quarter'>, Intl.DateTimeFormatOptions> = {
  year: { year: 'numeric' },
  month: { month: 'short', year: 'numeric' },
  week: { month: 'short', day: 'numeric' },
  day: { month: 'short', day: 'numeric', year: 'numeric' },
  hour: { month: 'short', day: 'numeric', hour: 'numeric' },
};

const formatters = new Map<string, Intl.DateTimeFormat>();
function dateFormat(options: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  const key = JSON.stringify(options);
  let formatter = formatters.get(key);
  if (!formatter) {
    formatter = new Intl.DateTimeFormat('en-US', { ...options, timeZone: 'UTC' });
    formatters.set(key, formatter);
  }
  return formatter;
}

/** The fiscal year a date falls in, named by the calendar year it ends in (Jul 2023–Jun 2024 = FY24). */
export function formatFiscalYear(ms: number, startMonth = 7): { label: string; range: string } {
  const d = new Date(ms);
  const endYear = d.getUTCMonth() + 1 >= startMonth && startMonth !== 1 ? d.getUTCFullYear() + 1 : d.getUTCFullYear();
  const startYear = startMonth === 1 ? endYear : endYear - 1;
  const month = dateFormat({ month: 'short' });
  const first = month.format(Date.UTC(startYear, startMonth - 1, 1));
  const last = month.format(Date.UTC(endYear, (startMonth + 10) % 12, 1));
  return { label: `FY${String(endYear % 100).padStart(2, '0')}`, range: `${first} ${startYear} to ${last} ${endYear}` };
}

export function formatDate(ms: number, granularity: TimeUnit, fiscalYearStartMonth = 7): string {
  if (granularity === 'fiscal-year') return formatFiscalYear(ms, fiscalYearStartMonth).label;
  if (granularity === 'quarter') {
    const d = new Date(ms);
    return `Q${Math.floor(d.getUTCMonth() / 3) + 1} ${d.getUTCFullYear()}`;
  }
  return dateFormat(FORMATS[granularity]).format(ms);
}

/** 'Mar 1 to Apr 4, 2024', or with both years when they differ. Never a hyphen between dates. */
export function formatDateRange(fromMs: number, toMs: number, granularity: TimeUnit = 'day'): string {
  if (granularity !== 'day') return `${formatDate(fromMs, granularity)} to ${formatDate(toMs, granularity)}`;
  const sameYear = new Date(fromMs).getUTCFullYear() === new Date(toMs).getUTCFullYear();
  const first = sameYear ? dateFormat({ month: 'short', day: 'numeric' }).format(fromMs) : formatDate(fromMs, 'day');
  return `${first} to ${formatDate(toMs, 'day')}`;
}

/** An elapsed time: '45 min', '18 h', '3.5 d', '41 d'. */
export function formatDuration(ms: number): string {
  const abs = Math.abs(ms);
  const sign = ms < 0 ? MINUS : '';
  const hours = abs / 3_600_000;
  if (hours < 1) return `${sign}${Math.round(abs / 60_000)} min`;
  if (hours < 48) return `${sign}${grouped(hours, hours < 10 ? 1 : 0).replace(/\.0$/, '')} h`;
  const days = hours / 24;
  return `${sign}${grouped(days, days < 10 ? 1 : 0).replace(/\.0$/, '')} d`;
}
