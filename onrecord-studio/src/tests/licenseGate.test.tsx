// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';

import LicenseGate from '../components/licensing/LicenseGate';
import type { LicenseStatus } from '../types';

const unavailable: LicenseStatus = {
  enforced: true,
  authorized: false,
  state: 'UNAVAILABLE',
  message: 'The App Store purchase could not be loaded.',
  bundleId: null,
  appVersion: null,
  originalAppVersion: null,
  appTransactionId: null,
  appId: null,
  environment: null,
  originalPurchaseDate: null,
  signedDate: null,
  revocationDate: null,
};

describe('LicenseGate', () => {
  afterEach(cleanup);

  it('keeps the workspace unavailable and offers StoreKit recovery', () => {
    const refresh = vi.fn();
    render(<LicenseGate status={unavailable} refreshing={false} error={null} onRefresh={refresh} />);

    expect(screen.getByText('App Store verification needed')).toBeTruthy();
    expect(screen.getByText(/workspace commands and local evidence remain locked/i)).toBeTruthy();
    fireEvent.click(screen.getByRole('button', { name: 'Verify purchase again' }));
    expect(refresh).toHaveBeenCalledOnce();
  });

  it('distinguishes a revoked purchase from a temporary verification outage', () => {
    render(
      <LicenseGate
        status={{ ...unavailable, state: 'REVOKED', message: 'Apple reports that this purchase was revoked.' }}
        refreshing={false}
        error={null}
        onRefresh={() => undefined}
      />,
    );

    expect(screen.getByText('Purchase no longer active')).toBeTruthy();
  });
});
