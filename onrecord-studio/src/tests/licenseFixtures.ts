import type { LicenseStatus } from '../types';

export function licenseStatus(overrides: Partial<LicenseStatus> = {}): LicenseStatus {
  return {
    enforced: true,
    authorized: false,
    state: 'SIGN_IN_REQUIRED',
    message: 'Sign in to use OnRecord Studio.',
    bundleId: 'com.onrecord.studio',
    appVersion: '1.0.0',
    originalAppVersion: null,
    appTransactionId: null,
    appId: null,
    environment: 'direct',
    originalPurchaseDate: null,
    signedDate: null,
    revocationDate: null,
    plan: null,
    trialEndsAt: null,
    daysLeft: null,
    accountEmail: null,
    signInProvider: null,
    entitlementExpiresAt: null,
    ...overrides,
  };
}
