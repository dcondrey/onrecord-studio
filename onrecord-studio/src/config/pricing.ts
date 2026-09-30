/** The only place price facts live. Every screen that states a price renders from this table. */
export interface PriceTier {
  minSeats: number;
  /** Inclusive. `null` means no upper bound. */
  maxSeats: number | null;
  usdPerSeatPerYear: number;
}

export const MIN_SEATS = 1;
export const MAX_SEATS = 999;

export const PRICE_TIERS: readonly PriceTier[] = [
  { minSeats: 1, maxSeats: 1, usdPerSeatPerYear: 79 },
  { minSeats: 2, maxSeats: 9, usdPerSeatPerYear: 69 },
  { minSeats: 10, maxSeats: null, usdPerSeatPerYear: 49 },
];

export const PRICE_TERMS = 'Per seat, billed yearly. Taxes extra. Coupons are accepted at checkout.';

export function clampSeats(value: number): number {
  if (!Number.isFinite(value)) return MIN_SEATS;
  return Math.min(MAX_SEATS, Math.max(MIN_SEATS, Math.trunc(value)));
}

export function tierForSeats(seats: number): PriceTier {
  const count = clampSeats(seats);
  const tier = PRICE_TIERS.find(
    (candidate) => count >= candidate.minSeats && (candidate.maxSeats === null || count <= candidate.maxSeats),
  );
  if (!tier) throw new Error(`No price tier covers ${count} seats.`);
  return tier;
}

export function yearlyTotalUsd(seats: number): number {
  const count = clampSeats(seats);
  return count * tierForSeats(count).usdPerSeatPerYear;
}

export function tierLabel(tier: PriceTier): string {
  if (tier.maxSeats === null) return `${tier.minSeats} or more seats`;
  if (tier.minSeats === tier.maxSeats) return `${tier.minSeats} seat`;
  return `${tier.minSeats} to ${tier.maxSeats} seats`;
}
