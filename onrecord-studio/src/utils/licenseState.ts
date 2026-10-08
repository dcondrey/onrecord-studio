import type { LicenseState, LicenseStatus } from '../types';

const ACCESS_STATES: ReadonlySet<LicenseState> = new Set(['DEVELOPMENT', 'VERIFIED', 'TRIAL', 'LICENSED']);

/**
 * The workspace opens only when the backend authorizes AND the state is one that grants access.
 * A status that says `authorized` next to TRIAL_EXPIRED or an unknown state is contradictory, and
 * a contradiction must not open evidence.
 */
export function grantsAccess(status: LicenseStatus | null): boolean {
  return status !== null && status.authorized && ACCESS_STATES.has(status.state);
}

/** The direct build stamps its own environment; the App Store build never does. */
export function isDirectChannel(status: LicenseStatus | null): boolean {
  return status?.environment === 'direct';
}

export function daysLeftLabel(daysLeft: number): string {
  if (daysLeft <= 0) return 'Ends today';
  return daysLeft === 1 ? '1 day left' : `${daysLeft} days left`;
}

/**
 * Whole days until `trialEndsAt`, rounded up like the backend, falling back to the backend's own
 * `daysLeft`. Absence stays null so the banner can say "trial active" instead of inventing a count.
 */
export function trialDaysLeft(status: LicenseStatus, now: number = Date.now()): number | null {
  if (status.trialEndsAt) {
    const end = Date.parse(status.trialEndsAt);
    if (Number.isFinite(end)) return Math.max(0, Math.ceil((end - now) / 86_400_000));
  }
  return status.daysLeft === null ? null : Math.max(0, status.daysLeft);
}
