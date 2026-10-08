import type { SqlQueryResult } from '../types';
import type { DiscoveryQuery } from './dataDiscovery';

export type QueryEntry = { query: DiscoveryQuery; result?: SqlQueryResult };

/** A numeric cell, or null when the cell is empty or not a finite number; never zero for absence. */
export const numberOrNull = (value: unknown): number | null => {
  if (value == null || value === '') return null;
  const parsed = Number(value);
  return Number.isFinite(parsed) ? parsed : null;
};

export const wholeNumber = (value: number) => Math.round(value).toLocaleString('en-US');
export const percent = (part: number, whole: number) => `${(100 * part / whole).toFixed(1)}%`;

export const money = (value: number) => {
  const size = Math.abs(value);
  return `${value < 0 ? '-' : ''}$${size >= 1_000_000_000 ? `${(size / 1_000_000_000).toFixed(2)} billion` : size >= 1_000_000 ? `${(size / 1_000_000).toFixed(1)} million` : wholeNumber(size)}`;
};

export const records = (result: SqlQueryResult) => result.rows.map((row) => Object.fromEntries(result.columns.map((name, index) => [name, row[index]])));

/** The rows of one query, or null when it returned none or was truncated (a partial result is not read). */
export const rowsOf = (results: QueryEntry[], id: string) => {
  const entry = results.find((item) => item.query.id === id);
  return entry?.result?.rows.length && !entry.result.truncated ? { query: entry.query, rows: records(entry.result) } : null;
};
