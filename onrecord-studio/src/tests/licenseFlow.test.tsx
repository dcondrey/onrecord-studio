// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import type { DeleteAccountReport, LicenseStatus, SignInReport } from '../types';
import { licenseStatus } from './licenseFixtures';

const ipc = vi.hoisted(() => ({ invokeCommand: vi.fn() }));
vi.mock('../ipc', async (importOriginal) => ({
  ...(await importOriginal<typeof import('../ipc')>()),
  invokeCommand: ipc.invokeCommand,
}));
vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => true, invoke: vi.fn() }));

import AccountMenu from '../components/account/AccountMenu';
import { LicenseProvider, useLicense } from '../components/licensing/LicenseProvider';
import LicenseSwitch from '../components/licensing/LicenseSwitch';
import TrialBanner from '../components/licensing/TrialBanner';

interface Deferred<T> {
  promise: Promise<T>;
  resolve: (value: T) => void;
  reject: (reason: unknown) => void;
}

function deferred<T>(): Deferred<T> {
  let resolve!: (value: T) => void;
  let reject!: (reason: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

function respond(handlers: Record<string, () => Promise<unknown>>) {
  ipc.invokeCommand.mockImplementation((command: string) => {
    const handler = handlers[command];
    if (!handler) throw new Error(`unexpected command ${command}`);
    return handler();
  });
}

function Probe() {
  const c = useLicense();
  if (!c) return null;
  return (
    <div>
      <p data-testid="state">{c.status?.state ?? 'loading'}</p>
      <p data-testid="notice">{c.signInNotice ?? ''}</p>
      <button onClick={() => c.signIn('google')}>in</button>
      <button onClick={() => c.signOut()}>out</button>
      <button onClick={() => c.activateThisDevice(true)}>activate</button>
    </div>
  );
}

const signedIn = licenseStatus({ state: 'LICENSED', authorized: true, accountEmail: 'a@example.org', signInProvider: 'google', plan: 'licensed' });
const signedOut = licenseStatus();

describe('license flow', () => {
  beforeEach(() => {
    ipc.invokeCommand.mockReset();
    window.sessionStorage.clear();
  });
  afterEach(cleanup);

  it('discards a sign-in that resolves after sign-out', async () => {
    const signIn = deferred<SignInReport>();
    respond({
      get_license_status: () => Promise.resolve(signedOut),
      begin_sign_in: () => signIn.promise,
      sign_out: () => Promise.resolve(signedOut),
    });
    render(<LicenseProvider><Probe /></LicenseProvider>);
    await waitFor(() => expect(screen.getByTestId('state').textContent).toBe('SIGN_IN_REQUIRED'));

    fireEvent.click(screen.getByText('in'));
    fireEvent.click(screen.getByText('out'));
    await waitFor(() => expect(ipc.invokeCommand).toHaveBeenCalledWith('sign_out', {}));
    await act(async () => {
      signIn.resolve({ outcome: 'signed_in', message: null, license: signedIn });
      await signIn.promise;
    });
    expect(screen.getByTestId('state').textContent).toBe('SIGN_IN_REQUIRED');
  });

  it('shows nothing for a cancelled sign-in and the returned message for a failed one', async () => {
    const outcomes: SignInReport[] = [
      { outcome: 'cancelled', message: null, license: signedOut },
      { outcome: 'failed', message: 'Provider said no.', license: signedOut },
    ];
    respond({
      get_license_status: () => Promise.resolve(signedOut),
      begin_sign_in: () => Promise.resolve(outcomes.shift() as SignInReport),
    });
    render(<LicenseProvider><Probe /></LicenseProvider>);
    await waitFor(() => expect(screen.getByTestId('state').textContent).toBe('SIGN_IN_REQUIRED'));
    fireEvent.click(screen.getByText('in'));
    await waitFor(() => expect(ipc.invokeCommand).toHaveBeenCalledWith('begin_sign_in', { provider: 'google' }));
    await act(async () => undefined);
    expect(screen.getByTestId('notice').textContent).toBe('');
    fireEvent.click(screen.getByText('in'));
    await waitFor(() => expect(screen.getByTestId('notice').textContent).toBe('Provider said no.'));
  });

  it('discards an activation that resolves after sign-out', async () => {
    const activation = deferred<LicenseStatus>();
    const limited = licenseStatus({ state: 'DEVICE_LIMIT_REACHED' });
    respond({
      get_license_status: () => Promise.resolve(limited),
      activate_this_device: () => activation.promise,
      sign_out: () => Promise.resolve(signedOut),
    });
    render(<LicenseProvider><Probe /></LicenseProvider>);
    await waitFor(() => expect(screen.getByTestId('state').textContent).toBe('DEVICE_LIMIT_REACHED'));

    fireEvent.click(screen.getByText('activate'));
    expect(ipc.invokeCommand).toHaveBeenCalledWith('activate_this_device', { confirmRevokeOldest: true });
    fireEvent.click(screen.getByText('out'));
    await waitFor(() => expect(ipc.invokeCommand).toHaveBeenCalledWith('sign_out', {}));
    await act(async () => {
      activation.resolve(signedIn);
      await activation.promise;
    });
    expect(screen.getByTestId('state').textContent).toBe('SIGN_IN_REQUIRED');
  });

  describe('access', () => {
    const cases: Array<[LicenseStatus['state'], boolean]> = [
      ['SIGN_IN_REQUIRED', false],
      ['TRIAL_EXPIRED', false],
      ['CLOCK_ROLLED_BACK', false],
      ['ENTITLEMENT_EXPIRED', false],
      ['REVOKED', false],
      ['UNVERIFIED', false],
      ['UNAVAILABLE', false],
      ['BUNDLE_MISMATCH', false],
      ['TRIAL', true],
      ['LICENSED', true],
    ];
    it.each(cases)('%s renders the app only when it grants access', async (state, opensApp) => {
      respond({ get_license_status: () => Promise.resolve(licenseStatus({ state, authorized: opensApp, daysLeft: 5 })) });
      render(
        <LicenseProvider>
          <LicenseSwitch><div data-testid="app" /></LicenseSwitch>
        </LicenseProvider>,
      );
      await waitFor(() => expect(screen.queryByText('Checking your license')).toBeNull());
      expect(screen.queryByTestId('app') !== null).toBe(opensApp);
      expect(screen.queryByRole('heading', { level: 1 }) !== null).toBe(!opensApp);
    });

    it('does not open the app while the status is still loading', () => {
      respond({ get_license_status: () => new Promise(() => undefined) });
      render(
        <LicenseProvider>
          <LicenseSwitch><div data-testid="app" /></LicenseSwitch>
        </LicenseProvider>,
      );
      expect(screen.queryByTestId('app')).toBeNull();
    });
  });

  describe('trial banner', () => {
    async function mountBanner(state: LicenseStatus['state'], daysLeft: number | null) {
      respond({
        get_license_status: () =>
          Promise.resolve(licenseStatus({ state, authorized: true, daysLeft, plan: state === 'TRIAL' ? 'trial' : 'licensed' })),
      });
      render(<LicenseProvider><TrialBanner /></LicenseProvider>);
      await act(async () => undefined);
    }

    it('counts days with correct pluralization and emphasizes the last three', async () => {
      await mountBanner('TRIAL', 30);
      expect(screen.getByRole('status').textContent).toMatch(/30 days left in your trial/);
      expect(screen.getByRole('status').textContent).not.toMatch(/ending soon/i);
      cleanup();
      await mountBanner('TRIAL', 1);
      expect(screen.getByRole('status').textContent).toMatch(/Trial ending soon: 1 day left in your trial/);
      cleanup();
      await mountBanner('TRIAL', 0);
      expect(screen.getByRole('status').textContent).toMatch(/Trial ending soon: Your trial ends today/);
    });

    it('never appears for a licensed account and is dismissible without blocking', async () => {
      await mountBanner('LICENSED', 30);
      expect(screen.queryByRole('status')).toBeNull();
      cleanup();
      await mountBanner('TRIAL', 10);
      fireEvent.click(screen.getByRole('button', { name: /Dismiss trial notice/ }));
      expect(screen.queryByRole('status')).toBeNull();
    });

    it('opens the upgrade section from the banner', async () => {
      await mountBanner('TRIAL', 10);
      fireEvent.click(screen.getByRole('button', { name: 'Upgrade' }));
      expect(screen.getByRole('dialog', { name: 'Upgrade OnRecord Studio' })).toBeTruthy();
      expect(screen.getByRole('table')).toBeTruthy();
    });
  });

  describe('delete account', () => {
    async function deleteWith(report: DeleteAccountReport) {
      respond({
        get_license_status: () => Promise.resolve(signedIn),
        delete_account: () => Promise.resolve(report),
      });
      render(<LicenseProvider><AccountMenu /></LicenseProvider>);
      await waitFor(() => screen.getByRole('button', { name: 'Account' }));
      fireEvent.click(screen.getByRole('button', { name: 'Account' }));
      fireEvent.click(screen.getByRole('button', { name: 'Delete account' }));
      const confirmation = screen.getByRole('group', { name: 'Confirm account deletion' });
      expect(confirmation.textContent).toMatch(/Local evidence on this Mac is not deleted/);
      expect(confirmation.textContent).toMatch(/trial already used on this device is not restored/);
      fireEvent.click(screen.getByRole('button', { name: 'Delete my account' }));
      await waitFor(() => expect(ipc.invokeCommand).toHaveBeenCalledWith('delete_account', {}));
      await act(async () => undefined);
    }

    it('never says deleted when the server step failed', async () => {
      await deleteWith({ remoteDeleted: false, signedOut: true, localCleared: true, notDone: ['Server unreachable.'], license: signedOut });
      const alert = screen.getByRole('alert');
      expect(alert.textContent).toMatch(/did not fully finish/);
      expect(alert.textContent).toMatch(/NOT deleted on the server/);
      expect(alert.textContent).not.toMatch(/was deleted on the server/);
      expect(alert.textContent).toMatch(/Server unreachable\./);
    });

    it('reports a complete deletion as done', async () => {
      await deleteWith({ remoteDeleted: true, signedOut: true, localCleared: true, notDone: [], license: signedOut });
      expect(screen.getByRole('status').textContent).toMatch(/Done: your license account was deleted on the server/);
    });
  });
});
