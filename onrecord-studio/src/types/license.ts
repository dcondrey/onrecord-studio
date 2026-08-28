export type LicenseState =
  | 'DEVELOPMENT'
  | 'VERIFIED'
  | 'REVOKED'
  | 'UNVERIFIED'
  | 'UNAVAILABLE'
  | 'BUNDLE_MISMATCH';

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
}
