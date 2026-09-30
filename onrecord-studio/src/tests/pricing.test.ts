import { readdirSync, readFileSync, statSync } from 'node:fs';
import { join } from 'node:path';
import { describe, expect, it } from 'vitest';

import { MAX_SEATS, PRICE_TIERS, clampSeats, tierForSeats, yearlyTotalUsd } from '../config/pricing';

function sourceFiles(dir: string): string[] {
  return readdirSync(dir).flatMap((name) => {
    const path = join(dir, name);
    return statSync(path).isDirectory() ? sourceFiles(path) : /\.tsx?$/.test(name) ? [path] : [];
  });
}

describe('pricing', () => {
  it('prices by tier at the boundaries and never leaves the seat range', () => {
    expect(tierForSeats(1).usdPerSeatPerYear).toBe(79);
    expect(tierForSeats(2).usdPerSeatPerYear).toBe(69);
    expect(tierForSeats(9).usdPerSeatPerYear).toBe(69);
    expect(tierForSeats(10).usdPerSeatPerYear).toBe(49);
    expect(tierForSeats(MAX_SEATS).usdPerSeatPerYear).toBe(49);
    expect(yearlyTotalUsd(9)).toBe(621);
    expect(yearlyTotalUsd(10)).toBe(490);
    expect([clampSeats(0), clampSeats(1000), clampSeats(Number.NaN), clampSeats(2.9)]).toEqual([1, 999, 1, 2]);
  });

  it('covers every seat count exactly once', () => {
    for (let seats = 1; seats <= MAX_SEATS; seats += 1) {
      const matches = PRICE_TIERS.filter(
        (tier) => seats >= tier.minSeats && (tier.maxSeats === null || seats <= tier.maxSeats),
      );
      expect(matches).toHaveLength(1);
    }
  });

  it('keeps every dollar amount in the licensing and account UI in the one constant file', () => {
    const offenders = ['components/licensing', 'components/account']
      .flatMap((dir) => sourceFiles(join(__dirname, '..', dir)))
      .filter((path) => /\$\d/.test(readFileSync(path, 'utf8')));
    expect(offenders).toEqual([]);
  });
});
