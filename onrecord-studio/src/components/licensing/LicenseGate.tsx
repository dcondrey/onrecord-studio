import { useEffect, useRef } from 'react';
import DeleteAccountResult from '../account/DeleteAccountResult';
import UpdatePrompt from '../updates/UpdatePrompt';
import Button from '../ui/Button';
import { PRODUCT_NAME } from '../../config/brand';
import { cn } from '../../utils/cn';
import { isDirectChannel } from '../../utils/licenseState';
import type { LicenseState, LicenseStatus } from '../../types';
import { type LicenseController } from './LicenseProvider';
import DeviceLimitPanel from './DeviceLimitPanel';
import UpgradePanel from './UpgradePanel';
import { PROVIDERS, ProviderMark, providerLabel } from './providers';

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
    heading: 'Signed in on too many Macs',
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

function sameWords(left: string, right: string): boolean {
  const normalize = (text: string) => text.trim().toLowerCase().split('.').join('');
  return normalize(left) === normalize(right);
}

/** The server counts "devices"; everything this screen says counts Macs. */
function inMacs(message: string): string {
  return message.split(' devices').join(' Macs').split(' device').join(' Mac');
}

/** The backend's own message, only when it says something the heading and sentence do not. */
function detailFor(status: LicenseStatus | null, copy: Copy): string | null {
  if (!status || !isDirectChannel(status) || status.state === 'SIGN_IN_REQUIRED') return null;
  const message = status.state === 'DEVICE_LIMIT_REACHED' ? inMacs(status.message) : status.message;
  if (!message.trim() || sameWords(message, copy.heading) || sameWords(message, copy.sentence)) return null;
  return message;
}

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
  // Not UNAVAILABLE or ENTITLEMENT_EXPIRED: those mean the server could not be reached, and a
  // paying subscriber who is merely offline must never be offered checkout, which could start a
  // second subscription for an account that already has one.
  const showUpgrade = direct && (status?.state === 'TRIAL_EXPIRED' || status?.state === 'REVOKED');
  const showVerify = status !== null && (!direct || status.state !== 'SIGN_IN_REQUIRED');
  const detail = detailFor(status, copy);
  // Verify is the primary action only when nothing else on the card is.
  const verifyIsPrimary = !showUpgrade && !showDeviceLimit;

  return (
    // A bug that denies every license check (like the Keychain allow-list bug fixed in beta.3)
    // can only reach an affected user through a signed update offered here, since LicenseSwitch
    // never mounts the workspace's own UpdatePrompt while the gate is showing.
    <main className="app-theme flex h-screen w-screen items-center justify-center overflow-y-auto bg-bg px-6 text-fg">
      <UpdatePrompt />
      <section className="my-8 max-h-[calc(100vh-4rem)] w-full max-w-lg overflow-y-auto rounded-2xl border border-line bg-panel p-8 shadow-dialog" aria-labelledby="license-gate-heading">
        <div className="flex items-center gap-2">
          <img src="/brand-mark.png" alt="" className="h-6 w-6 rounded-md" />
          <p className="text-heading text-fg">{PRODUCT_NAME}</p>
        </div>
        <h1 id="license-gate-heading" ref={headingRef} tabIndex={-1} className="mt-6 text-title text-fg outline-none">
          {copy.heading}
        </h1>
        <p className="mt-2 text-body text-fg-secondary">{copy.sentence}</p>
        {detail ? <p className="mt-2 text-caption text-muted">{detail}</p> : null}
        {direct && status?.accountEmail && !showDeviceLimit ? (
          <p className="mt-3 flex flex-wrap items-center gap-x-2 text-caption text-fg-secondary">
            <span>
              Signed in as {status.accountEmail}
              {status.signInProvider ? ` with ${providerLabel(status.signInProvider)}` : ''}.
            </span>
            <Button variant="link" disabled={busy} onClick={controller.signOut} className="text-caption">Sign out</Button>
          </p>
        ) : null}
        {error ? (
          <p role="alert" className="mt-4 rounded-lg border border-danger/30 bg-danger/10 px-4 py-3 text-body text-danger">
            {error}
          </p>
        ) : null}
        {signInNotice ? (
          <p role="alert" className="mt-4 rounded-lg border border-danger/30 bg-danger/10 px-4 py-3 text-body text-danger">
            Sign-in did not complete. {signInNotice}
          </p>
        ) : null}
        {signInProvider ? (
          <p role="status" className="mt-4 text-body text-fg-secondary">Waiting for {providerLabel(signInProvider)} sign-in in your browser window…</p>
        ) : null}
        <DeleteAccountResult report={controller.deleteReport} error={controller.deleteError} onDismiss={controller.dismissDeleteResult} />

        {showSignIn ? (
          <div className="mt-6 flex flex-col gap-2" role="group" aria-label="Sign in with">
            {PROVIDERS.map((provider) => (
              <Button
                key={provider.id}
                size="lg"
                disabled={busy}
                aria-busy={signInProvider === provider.id}
                onClick={() => controller.signIn(provider.id)}
                className={cn('w-full', provider.id === 'apple' && 'border-transparent bg-fg text-bg hover:bg-fg/90')}
              >
                <ProviderMark provider={provider.id} />
                {signInProvider === provider.id ? `Signing in with ${provider.label}…` : `Continue with ${provider.label}`}
              </Button>
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
          <div className="mt-5 space-y-2 rounded-lg bg-raised px-4 py-3 text-caption text-fg-secondary">
            <p>Open the Mac App Store and sign in with the Apple Account that purchased this app.</p>
            <p>This is a one-time paid app; no subscription or separate license key is required.</p>
          </div>
        ) : null}

        {showVerify || loading ? (
          <Button
            variant={verifyIsPrimary ? 'primary' : showUpgrade ? 'link' : 'secondary'}
            size="lg"
            disabled={busy || (loading && !error)}
            onClick={controller.refresh}
            className={showUpgrade ? 'mx-auto mt-4 flex' : 'mt-6 w-full'}
          >
            {refreshing || (loading && !error)
              ? direct || loading
                ? 'Checking license…'
                : 'Checking purchase…'
              : direct || loading
                ? 'Verify license again'
                : 'Verify purchase again'}
          </Button>
        ) : null}
        {refreshing ? <p role="status" className="sr-only">Checking license</p> : null}
        {showVerify || loading ? (
          <p className="mt-4 text-center text-caption text-muted">
            {direct || loading
              ? 'Your local evidence stays on this Mac.'
              : 'Workspace commands and local evidence remain locked until StoreKit verifies the paid app transaction.'}
          </p>
        ) : null}
      </section>
    </main>
  );
}
