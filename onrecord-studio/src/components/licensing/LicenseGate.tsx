import { useEffect, useRef } from 'react';
import DeleteAccountResult from '../account/DeleteAccountResult';
import { PRODUCT_BYLINE, PRODUCT_NAME } from '../../config/brand';
import { isDirectChannel } from '../../utils/licenseState';
import type { LicenseState, LicenseStatus } from '../../types';
import { type LicenseController, type SignInProvider } from './LicenseProvider';
import DeviceLimitPanel from './DeviceLimitPanel';
import UpgradePanel from './UpgradePanel';

const PROVIDERS: ReadonlyArray<{ id: SignInProvider; label: string }> = [
  { id: 'google', label: 'Google' },
  { id: 'apple', label: 'Apple' },
  { id: 'linkedin', label: 'LinkedIn' },
  { id: 'github', label: 'GitHub' },
];

interface Copy {
  heading: string;
  sentence: string;
}

function storeKitCopy(status: LicenseStatus | null): Copy {
  switch (status?.state) {
    case 'REVOKED':
      return { heading: 'Purchase no longer active', sentence: status.message };
    case 'UNVERIFIED':
    case 'BUNDLE_MISMATCH':
      return { heading: 'This copy could not be verified', sentence: status.message };
    case 'UNAVAILABLE':
      return { heading: 'App Store verification needed', sentence: status.message };
    default:
      return {
        heading: 'Verifying App Store purchase',
        sentence: status?.message ?? 'Checking Apple’s signed app transaction before opening the evidence workspace.',
      };
  }
}

const DIRECT_COPY: Partial<Record<LicenseState, Copy>> = {
  SIGN_IN_REQUIRED: {
    heading: 'Sign in to continue',
    sentence: 'OnRecord Studio needs an account to start your free trial or open your license.',
  },
  TRIAL_EXPIRED: {
    heading: 'Your free trial has ended',
    sentence: 'Upgrade to keep using the evidence workspace. Your local evidence on this Mac is unchanged.',
  },
  CLOCK_ROLLED_BACK: {
    heading: 'This Mac’s clock looks wrong',
    sentence:
      'The date on this Mac is earlier than a time OnRecord Studio has already seen. Correct the date and time, then verify your license again.',
  },
  ENTITLEMENT_EXPIRED: {
    heading: 'License needs to be verified online',
    sentence: 'Cannot reach the license server. Your last verified license has expired.',
  },
  DEVICE_LIMIT_REACHED: {
    heading: 'This account is active on the maximum number of Macs',
    sentence:
      'Your account is already signed in on the most Macs it allows. To use this Mac, sign your oldest Mac out of the account, or sign out here.',
  },
  REVOKED: {
    heading: 'License no longer active',
    sentence: 'The license server reports that this license is no longer active.',
  },
  UNVERIFIED: {
    heading: 'This license could not be verified',
    sentence: 'The stored license did not pass verification on this Mac. Verify again, or sign in again.',
  },
  BUNDLE_MISMATCH: {
    heading: 'This copy could not be verified',
    sentence: 'The stored license does not belong to this app. Sign in again to get a license for this copy.',
  },
  UNAVAILABLE: {
    heading: 'License could not be checked',
    sentence: 'OnRecord Studio could not check your license. If this Mac is offline, connect to the internet and verify again.',
  },
};

function copyFor(status: LicenseStatus | null): Copy {
  if (status === null) {
    return { heading: 'Checking your license', sentence: 'Checking your license before opening the evidence workspace.' };
  }
  if (!isDirectChannel(status)) return storeKitCopy(status);
  return (
    DIRECT_COPY[status.state] ?? {
      heading: 'License could not be checked',
      sentence: status.message,
    }
  );
}

const SHOWS_SIGN_IN: ReadonlySet<LicenseState> = new Set(['SIGN_IN_REQUIRED', 'UNVERIFIED', 'BUNDLE_MISMATCH']);

export default function LicenseGate({ controller }: { controller: LicenseController }) {
  const { status, error, refreshing, signInProvider, signInNotice, signingOut, activating } = controller;
  const loading = status === null;
  const direct = isDirectChannel(status);
  const copy = copyFor(status);
  const headingRef = useRef<HTMLHeadingElement>(null);
  const busy = refreshing || signInProvider !== null || signingOut || activating || controller.deleting;

  useEffect(() => {
    headingRef.current?.focus();
  }, [copy.heading]);

  const showSignIn = direct && status !== null && SHOWS_SIGN_IN.has(status.state);
  const showDeviceLimit = direct && status?.state === 'DEVICE_LIMIT_REACHED';
  const showUpgrade = direct && status?.state === 'TRIAL_EXPIRED';
  const showVerify = status !== null && (!direct || status.state !== 'SIGN_IN_REQUIRED');
  const detail = direct && status && status.message !== copy.sentence && status.state !== 'SIGN_IN_REQUIRED'
    ? status.message
    : null;

  return (
    <main className="app-theme flex h-screen w-screen items-center justify-center overflow-y-auto bg-canvas px-6 text-fg">
      <section className="my-auto w-full max-w-lg rounded-2xl border border-line-strong bg-surface/70 p-8 shadow-2xl" aria-labelledby="license-gate-heading">
        <p className="text-label leading-4 font-semibold uppercase tracking-[0.24em] text-accent">{PRODUCT_NAME}</p>
        <p className="mt-1 text-caption text-muted">{PRODUCT_BYLINE}</p>
        <h1 id="license-gate-heading" ref={headingRef} tabIndex={-1} className="mt-3 text-2xl font-semibold outline-none">
          {copy.heading}
        </h1>
        <p className="mt-4 text-sm leading-6 text-fg-secondary">{copy.sentence}</p>
        {detail ? <p className="mt-2 text-label leading-5 text-muted">Detail: {detail}</p> : null}
        {direct && status?.accountEmail && !showDeviceLimit ? (
          <p className="mt-3 text-label leading-4 text-fg-secondary">
            Signed in as {status.accountEmail}
            {status.signInProvider ? ` with ${status.signInProvider}` : ''}.{' '}
            <button
              type="button"
              disabled={busy}
              onClick={controller.signOut}
              className="focus-ring inline-flex min-h-[24px] items-center rounded px-1.5 font-semibold text-accent underline disabled:opacity-50"
            >
              Sign out
            </button>
          </p>
        ) : null}
        {error ? (
          <p role="alert" className="mt-4 rounded-lg border border-danger/30 bg-danger/10 px-4 py-3 text-sm text-danger">
            {error}
          </p>
        ) : null}
        {signInNotice ? (
          <p role="alert" className="mt-4 rounded-lg border border-danger/30 bg-danger/10 px-4 py-3 text-sm text-danger">
            Sign-in did not complete. {signInNotice}
          </p>
        ) : null}
        {signInProvider ? (
          <p role="status" className="mt-4 text-sm text-fg-secondary">Waiting for {providerLabel(signInProvider)} sign-in in your browser window...</p>
        ) : null}
        <DeleteAccountResult report={controller.deleteReport} error={controller.deleteError} onDismiss={controller.dismissDeleteResult} />

        {showSignIn ? (
          <div className="mt-6 grid grid-cols-2 gap-2" role="group" aria-label="Sign in with">
            {PROVIDERS.map((provider) => (
              <button
                key={provider.id}
                type="button"
                disabled={busy}
                aria-busy={signInProvider === provider.id}
                onClick={() => controller.signIn(provider.id)}
                className="focus-ring rounded-lg border border-control px-3 py-2.5 text-sm font-semibold text-fg hover:bg-raised disabled:cursor-wait disabled:opacity-50"
              >
                {signInProvider === provider.id ? `Signing in with ${provider.label}...` : `Continue with ${provider.label}`}
              </button>
            ))}
          </div>
        ) : null}

        {showDeviceLimit ? (
          <DeviceLimitPanel
            busy={busy}
            activating={activating}
            onActivate={controller.activateThisDevice}
            onSignOut={controller.signOut}
          />
        ) : null}

        {showUpgrade ? <UpgradePanel checkout={controller.checkout} onStartCheckout={controller.startCheckout} /> : null}

        {!loading && !direct ? (
          <div className="mt-5 space-y-2 rounded-lg bg-canvas/70 px-4 py-3 text-label leading-5 text-muted">
            <p>Open the Mac App Store and sign in with the Apple Account that purchased this app.</p>
            <p>This is a one-time paid app; no subscription or separate license key is required.</p>
          </div>
        ) : null}

        {showVerify || loading ? (
          <button
            type="button"
            disabled={busy || (loading && !error)}
            onClick={controller.refresh}
            className="focus-ring mt-6 w-full rounded-lg bg-accent px-4 py-3 text-sm font-semibold text-canvas transition hover:bg-accent disabled:cursor-wait disabled:opacity-50"
          >
            {refreshing || (loading && !error)
              ? direct || loading
                ? 'Checking license...'
                : 'Checking purchase...'
              : direct || loading
                ? 'Verify license again'
                : 'Verify purchase again'}
          </button>
        ) : null}
        {refreshing ? <p role="status" className="sr-only">Checking license</p> : null}
        <p className="mt-4 text-center text-label leading-4 text-muted">
          {loading
            ? 'Workspace commands stay locked until the license check finishes. Local evidence on this Mac is not deleted.'
            : direct
              ? 'Workspace commands stay locked until a license is verified. Local evidence on this Mac is not deleted.'
              : 'Workspace commands and local evidence remain locked until StoreKit verifies the paid app transaction.'}
        </p>
      </section>
    </main>
  );
}

function providerLabel(id: SignInProvider): string {
  return PROVIDERS.find((provider) => provider.id === id)?.label ?? id;
}
