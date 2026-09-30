// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import LicenseGate from '../components/licensing/LicenseGate';
import type { LicenseController } from '../components/licensing/LicenseProvider';
import { PRICE_TIERS } from '../config/pricing';
import type { LicenseState } from '../types';
import { grantsAccess } from '../utils/licenseState';
import { licenseStatus } from './licenseFixtures';

function controller(overrides: Partial<LicenseController> = {}): LicenseController {
  return {
    status: licenseStatus(),
    error: null,
    refreshing: false,
    signInProvider: null,
    signInNotice: null,
    signingOut: false,
    activating: false,
    deleting: false,
    deleteReport: null,
    deleteError: null,
    checkout: { busy: false, error: null, url: null },
    refresh: vi.fn(),
    signIn: vi.fn(),
    signOut: vi.fn(),
    activateThisDevice: vi.fn(),
    deleteAccount: vi.fn(),
    dismissDeleteResult: vi.fn(),
    startCheckout: vi.fn(),
    ...overrides,
  };
}

const UNAUTHORIZED: LicenseState[] = [
  'SIGN_IN_REQUIRED',
  'TRIAL_EXPIRED',
  'CLOCK_ROLLED_BACK',
  'ENTITLEMENT_EXPIRED',
  'REVOKED',
  'UNVERIFIED',
  'UNAVAILABLE',
  'BUNDLE_MISMATCH',
  'DEVICE_LIMIT_REACHED',
];

describe('LicenseGate, direct build', () => {
  afterEach(cleanup);

  it.each(UNAUTHORIZED)('%s shows a blocking heading and one sentence', (state) => {
    render(<LicenseGate controller={controller({ status: licenseStatus({ state }) })} />);
    expect(screen.getByRole('heading', { level: 1 }).textContent).not.toBe('');
    expect(screen.getByRole('main')).toBeTruthy();
    expect(document.activeElement).toBe(screen.getByRole('heading', { level: 1 }));
  });

  it('offers all four providers, each calling begin_sign_in with its own id', () => {
    const c = controller();
    render(<LicenseGate controller={c} />);
    for (const [label, id] of [['Google', 'google'], ['Apple', 'apple'], ['LinkedIn', 'linkedin'], ['GitHub', 'github']]) {
      fireEvent.click(screen.getByRole('button', { name: `Continue with ${label}` }));
      expect(c.signIn).toHaveBeenLastCalledWith(id);
    }
  });

  it('shows busy on the clicked provider only and disables the others', () => {
    render(<LicenseGate controller={controller({ signInProvider: 'apple' })} />);
    expect(screen.getByRole('button', { name: 'Signing in with Apple...' }).getAttribute('aria-busy')).toBe('true');
    expect((screen.getByRole('button', { name: 'Continue with Google' }) as HTMLButtonElement).disabled).toBe(true);
    expect(screen.getByRole('status').textContent).toMatch(/Apple/);
  });

  it('shows nothing alarming for a cancelled sign-in and the message for a failed one', () => {
    const { rerender } = render(<LicenseGate controller={controller({ signInNotice: null })} />);
    expect(screen.queryByRole('alert')).toBeNull();
    rerender(<LicenseGate controller={controller({ signInNotice: 'Provider rejected the request.' })} />);
    expect(screen.getByRole('alert').textContent).toMatch(/Provider rejected the request\./);
  });

  it('states the offline expiry honestly and never blames credentials', () => {
    render(<LicenseGate controller={controller({ status: licenseStatus({ state: 'ENTITLEMENT_EXPIRED' }) })} />);
    expect(screen.getByText('Cannot reach the license server. Your last verified license has expired.')).toBeTruthy();
    expect(document.body.textContent).not.toMatch(/password|credential|incorrect/i);
    expect(screen.getByRole('button', { name: 'Verify license again' })).toBeTruthy();
  });

  it('TRIAL_EXPIRED offers the price table and a seats-based checkout, other states do not', () => {
    const c = controller({ status: licenseStatus({ state: 'TRIAL_EXPIRED' }) });
    const { rerender } = render(<LicenseGate controller={c} />);
    const table = screen.getByRole('table');
    for (const tier of PRICE_TIERS) expect(within(table).getByText(`$${tier.usdPerSeatPerYear}`)).toBeTruthy();
    expect(screen.getByText(/Taxes extra\. Coupons are accepted at checkout\./)).toBeTruthy();
    fireEvent.change(screen.getByLabelText('Seats'), { target: { value: '10' } });
    expect(screen.getByText(/10 seats at \$49 = \$490 per year/)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Continue to checkout' }));
    expect(c.startCheckout).toHaveBeenCalledWith(10);
    fireEvent.change(screen.getByLabelText('Seats'), { target: { value: '5000' } });
    fireEvent.click(screen.getByRole('button', { name: 'Continue to checkout' }));
    expect(c.startCheckout).toHaveBeenLastCalledWith(999);

    rerender(<LicenseGate controller={controller({ status: licenseStatus({ state: 'SIGN_IN_REQUIRED' }) })} />);
    expect(screen.queryByRole('table')).toBeNull();
  });

  it('runs the verify action', () => {
    const c = controller({ status: licenseStatus({ state: 'TRIAL_EXPIRED' }) });
    render(<LicenseGate controller={c} />);
    fireEvent.click(screen.getByRole('button', { name: 'Verify license again' }));
    expect(c.refresh).toHaveBeenCalledOnce();
  });

  it('shows an unsuccessful account deletion as not done', () => {
    render(
      <LicenseGate
        controller={controller({
          deleteReport: {
            remoteDeleted: false,
            signedOut: true,
            localCleared: true,
            notDone: ['The server could not be reached.'],
            license: licenseStatus(),
          },
        })}
      />,
    );
    const alert = screen.getByRole('alert');
    expect(alert.textContent).toMatch(/NOT deleted on the server/);
    expect(alert.textContent).not.toMatch(/was deleted on the server/);
    expect(alert.textContent).toMatch(/The server could not be reached\./);
  });
});

describe('LicenseGate, device limit', () => {
  afterEach(cleanup);
  const limited = () => licenseStatus({ state: 'DEVICE_LIMIT_REACHED', accountEmail: 'a@example.org' });

  it('explains the limit and never revokes without a second, explicit confirmation', () => {
    const c = controller({ status: limited() });
    render(<LicenseGate controller={c} />);
    expect(screen.getByRole('heading', { level: 1 }).textContent).toMatch(/maximum number of Macs/);
    expect(grantsAccess(limited())).toBe(false);

    fireEvent.click(screen.getByRole('button', { name: 'Use this Mac and sign out my oldest Mac' }));
    expect(c.activateThisDevice).not.toHaveBeenCalled();
    expect(screen.getByText('Sign out your oldest Mac and use this one?')).toBeTruthy();

    fireEvent.click(screen.getByRole('button', { name: 'Yes, sign out my oldest Mac' }));
    expect(c.activateThisDevice).toHaveBeenCalledOnce();
    expect(c.activateThisDevice).toHaveBeenCalledWith(true);
  });

  it('cancel returns to the first step without calling the command', () => {
    const c = controller({ status: limited() });
    render(<LicenseGate controller={c} />);
    fireEvent.click(screen.getByRole('button', { name: 'Use this Mac and sign out my oldest Mac' }));
    fireEvent.click(screen.getByRole('button', { name: 'Cancel' }));
    expect(screen.getByRole('button', { name: 'Use this Mac and sign out my oldest Mac' })).toBeTruthy();
    expect(c.activateThisDevice).not.toHaveBeenCalled();
  });

  it('offers a single Sign out that calls signOut, and disables both while busy', () => {
    const c = controller({ status: limited() });
    const { rerender } = render(<LicenseGate controller={c} />);
    fireEvent.click(screen.getByRole('button', { name: 'Sign out' }));
    expect(c.signOut).toHaveBeenCalledOnce();
    rerender(<LicenseGate controller={{ ...c, activating: true }} />);
    for (const name of ['Use this Mac and sign out my oldest Mac', 'Sign out']) {
      expect((screen.getByRole('button', { name }) as HTMLButtonElement).disabled).toBe(true);
    }
  });
});

describe('LicenseGate, partial account deletion', () => {
  afterEach(cleanup);
  const show = (flags: { remoteDeleted: boolean; signedOut: boolean; localCleared: boolean }) =>
    render(
      <LicenseGate
        controller={controller({ deleteReport: { ...flags, notDone: [], license: licenseStatus() } })}
      />,
    );

  it('reports a server delete whose local clear failed without claiming completion', () => {
    show({ remoteDeleted: true, signedOut: true, localCleared: false });
    const alert = screen.getByRole('alert');
    expect(alert.textContent).toMatch(/did not fully finish/);
    expect(alert.textContent).toMatch(/deleted on the server/);
    expect(alert.textContent).toMatch(/NOT fully removed/);
    expect(alert.textContent).toMatch(/Done: this Mac was signed out/);
  });

  it('reports a failed sign-out separately', () => {
    show({ remoteDeleted: true, signedOut: false, localCleared: true });
    expect(screen.getByRole('alert').textContent).toMatch(/NOT signed out/);
  });

  it('claims completion only when all three parts are done', () => {
    show({ remoteDeleted: true, signedOut: true, localCleared: true });
    const status = screen.getByRole('status');
    expect(status.textContent).toMatch(/Account deletion finished/);
    expect(status.textContent).not.toMatch(/NOT/);
  });
});

describe('LicenseGate, App Store build', () => {
  afterEach(cleanup);

  it('keeps the StoreKit copy and recovery action', () => {
    const c = controller({
      status: licenseStatus({
        environment: 'production',
        state: 'UNAVAILABLE',
        message: 'The App Store purchase could not be loaded.',
      }),
    });
    render(<LicenseGate controller={c} />);
    expect(screen.getByText('App Store verification needed')).toBeTruthy();
    expect(screen.getByText(/workspace commands and local evidence remain locked/i)).toBeTruthy();
    expect(screen.queryByRole('button', { name: /Continue with/ })).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'Verify purchase again' }));
    expect(c.refresh).toHaveBeenCalledOnce();
  });

  it('distinguishes a revoked purchase from a temporary outage', () => {
    render(
      <LicenseGate
        controller={controller({
          status: licenseStatus({ environment: 'production', state: 'REVOKED', message: 'Apple reports that this purchase was revoked.' }),
        })}
      />,
    );
    expect(screen.getByText('Purchase no longer active')).toBeTruthy();
  });
});

describe('grantsAccess', () => {
  it('opens only for access states that the backend also authorized', () => {
    const granted: LicenseState[] = ['DEVELOPMENT', 'VERIFIED', 'TRIAL', 'LICENSED'];
    for (const state of granted) {
      expect(grantsAccess(licenseStatus({ state, authorized: true }))).toBe(true);
      expect(grantsAccess(licenseStatus({ state, authorized: false }))).toBe(false);
    }
    for (const state of UNAUTHORIZED) {
      expect(grantsAccess(licenseStatus({ state, authorized: false }))).toBe(false);
      // A contradictory status fails closed.
      expect(grantsAccess(licenseStatus({ state, authorized: true }))).toBe(false);
    }
    expect(grantsAccess(null)).toBe(false);
  });
});
