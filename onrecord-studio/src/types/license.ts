export type LicenseState =
  | 'DEVELOPMENT'
  | 'VERIFIED'
  | 'REVOKED'
  | 'UNVERIFIED'
  | 'UNAVAILABLE'
  | 'BUNDLE_MISMATCH'
  | 'SIGN_IN_REQUIRED'
  | 'TRIAL'
  | 'TRIAL_EXPIRED'
  | 'LICENSED'
  | 'CLOCK_ROLLED_BACK'
  | 'ENTITLEMENT_EXPIRED'
  | 'DEVICE_LIMIT_REACHED';

export type LicensePlan = 'trial' | 'licensed';

export interface LicenseStatus {
  enforced: boolean;
  authorized: boolean;
  state: LicenseState;
  message: string;
  bundleId: string | null;
  appVersion: string | null;
  originalAppVersion: string | null;
  appTransactionId: string | null;
  appId: number | null;
  environment: string | null;
  originalPurchaseDate: string | null;
  signedDate: string | null;
  revocationDate: string | null;
  plan: LicensePlan | null;
  /** RFC 3339 UTC. Present exactly when the state is TRIAL or TRIAL_EXPIRED. */
  trialEndsAt: string | null;
  daysLeft: number | null;
  accountEmail: string | null;
  signInProvider: string | null;
  entitlementExpiresAt: string | null;
}

export interface AccountStatus {
  /** False in builds that have no account system (App Store, development). */
  available: boolean;
  signedIn: boolean;
  email: string | null;
  provider: string | null;
}

export type SignInOutcome = 'signed_in' | 'cancelled' | 'failed';

export interface SignInReport {
  outcome: SignInOutcome;
  message: string | null;
  license: LicenseStatus;
}

export interface DeleteAccountReport {
  remoteDeleted: boolean;
  signedOut: boolean;
  localCleared: boolean;
  notDone: string[];
  license: LicenseStatus;
}

export interface CheckoutSession {
  url: string;
}

export interface SeatEntry {
  email: string;
  invitedAt: string;
  claimedAt: string | null;
  revokedAt: string | null;
}

export interface SeatStatus {
  subscriptionActive: boolean;
  quantity: number;
  usedSeats: number;
  seats: SeatEntry[];
}
