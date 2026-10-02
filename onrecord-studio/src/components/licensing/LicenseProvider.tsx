import { createContext, useCallback, useContext, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { isTauri } from '@tauri-apps/api/core';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { DeleteAccountReport, LicenseStatus } from '../../types';

export type SignInProvider = 'google' | 'apple' | 'linkedin' | 'github';

export interface CheckoutState {
  busy: boolean;
  error: string | null;
  url: string | null;
}

export interface LicenseController {
  status: LicenseStatus | null;
  /** A failed status load or action; never a statement about the user's credentials. */
  error: string | null;
  refreshing: boolean;
  signInProvider: SignInProvider | null;
  /** The message a failed sign-in returned. A cancelled sign-in leaves this null. */
  signInNotice: string | null;
  signingOut: boolean;
  activating: boolean;
  deleting: boolean;
  deleteReport: DeleteAccountReport | null;
  deleteError: string | null;
  checkout: CheckoutState;
  refresh: () => void;
  signIn: (provider: SignInProvider) => void;
  signOut: () => void;
  /** Only `true` from an explicit user confirmation: it signs the account's oldest Mac out. */
  activateThisDevice: (confirmRevokeOldest: boolean) => void;
  deleteAccount: () => void;
  dismissDeleteResult: () => void;
  startCheckout: (seats: number) => void;
}

const LicenseContext = createContext<LicenseController | null>(null);

/** Null outside a provider, so surfaces that are optional (the banner, the menu) render nothing. */
export function useLicense(): LicenseController | null {
  return useContext(LicenseContext);
}

const FOCUS_RECHECK_MILLISECONDS = 30_000;

const BROWSER_DEVELOPMENT_LICENSE: LicenseStatus = {
  enforced: false,
  authorized: true,
  state: 'DEVELOPMENT',
  message: 'Browser preview uses development access.',
  bundleId: 'com.onrecord.studio',
  appVersion: null,
  originalAppVersion: null,
  appTransactionId: null,
  appId: null,
  environment: 'browser',
  originalPurchaseDate: null,
  signedDate: null,
  revocationDate: null,
  plan: null,
  trialEndsAt: null,
  daysLeft: null,
  accountEmail: null,
  signInProvider: null,
  entitlementExpiresAt: null,
};

// After a purchase in the browser the app must ask the server, not re-read local state.
const REFRESH_ON_FOCUS = new Set(['TRIAL_EXPIRED', 'ENTITLEMENT_EXPIRED']);

export function LicenseProvider({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<LicenseStatus | null>(() => (isTauri() ? null : BROWSER_DEVELOPMENT_LICENSE));
  const [error, setError] = useState<string | null>(null);
  const [refreshing, setRefreshing] = useState(false);
  const [signInProvider, setSignInProvider] = useState<SignInProvider | null>(null);
  const [signInNotice, setSignInNotice] = useState<string | null>(null);
  const [signingOut, setSigningOut] = useState(false);
  const [activating, setActivating] = useState(false);
  const [deleting, setDeleting] = useState(false);
  const [deleteReport, setDeleteReport] = useState<DeleteAccountReport | null>(null);
  const [deleteError, setDeleteError] = useState<string | null>(null);
  const [checkout, setCheckout] = useState<CheckoutState>({ busy: false, error: null, url: null });

  // One gate for everything that writes the license status: a result that arrives after a newer
  // action (sign-out, another sign-in) describes a state that no longer exists and must be dropped.
  const licenseGate = useLatestRequestGate();
  const checkoutGate = useLatestRequestGate();
  const busyRef = useRef(false);
  const statusRef = useRef<LicenseStatus | null>(status);
  const lastCheckRef = useRef(0);

  const publishStatus = useCallback((next: LicenseStatus) => {
    statusRef.current = next;
    setStatus(next);
  }, []);

  const clearBusy = useCallback(() => {
    busyRef.current = false;
    setRefreshing(false);
    setSignInProvider(null);
    setSigningOut(false);
    setActivating(false);
    setDeleting(false);
  }, []);

  const begin = useCallback(() => {
    checkoutGate.invalidate();
    setCheckout({ busy: false, error: null, url: null });
    setError(null);
    busyRef.current = true;
    return licenseGate.begin();
  }, [checkoutGate, licenseGate]);

  // Shared gate-checking plumbing for every licenseGate-backed action: begin a token, run the
  // IPC call, and drop the result/error/busy-clear if a newer action has since invalidated it.
  const runGated = useCallback(
    <T,>(
      action: () => Promise<T>,
      onSuccess: (result: T) => void,
      options?: { onStart?: () => void; onError?: (failure: unknown) => void },
    ) => {
      const token = begin();
      options?.onStart?.();
      void action()
        .then((result) => {
          if (licenseGate.isCurrent(token)) onSuccess(result);
        })
        .catch((failure: unknown) => {
          if (!licenseGate.isCurrent(token)) return;
          if (options?.onError) {
            options.onError(failure);
          } else {
            setError(formatNativeError(failure));
          }
        })
        .finally(() => {
          if (licenseGate.isCurrent(token)) clearBusy();
        });
    },
    [begin, clearBusy, licenseGate],
  );

  const load = useCallback(
    (command: 'get_license_status' | 'refresh_license', showBusy: boolean) => {
      if (!isTauri()) return;
      runGated(() => invokeCommand(command, {}), publishStatus, {
        onStart: () => {
          if (showBusy) setRefreshing(true);
        },
      });
    },
    [runGated, publishStatus],
  );

  useEffect(() => {
    load('get_license_status', false);
  }, [load]);

  useEffect(() => {
    if (!isTauri()) return;
    const recheck = () => {
      const now = Date.now();
      if (busyRef.current || now - lastCheckRef.current < FOCUS_RECHECK_MILLISECONDS) return;
      lastCheckRef.current = now;
      const state = statusRef.current?.state;
      load(state && REFRESH_ON_FOCUS.has(state) ? 'refresh_license' : 'get_license_status', false);
    };
    window.addEventListener('focus', recheck);
    return () => window.removeEventListener('focus', recheck);
  }, [load]);

  const refresh = useCallback(() => {
    if (busyRef.current) return;
    setSignInNotice(null);
    load('refresh_license', true);
  }, [load]);

  const signIn = useCallback(
    (provider: SignInProvider) => {
      if (!isTauri() || busyRef.current) return;
      runGated(
        () => invokeCommand('begin_sign_in', { provider }),
        (report) => {
          publishStatus(report.license);
          // A cancelled sign-in is the user's choice, not a fault: nothing is shown for it.
          if (report.outcome === 'failed') setSignInNotice(report.message ?? 'Sign-in did not complete.');
        },
        {
          onStart: () => {
            setSignInNotice(null);
            setSignInProvider(provider);
          },
          onError: (failure) => setSignInNotice(formatNativeError(failure)),
        },
      );
    },
    [runGated, publishStatus],
  );

  const signOut = useCallback(() => {
    if (!isTauri()) return;
    runGated(() => invokeCommand('sign_out', {}), publishStatus, {
      onStart: () => {
        setSignInNotice(null);
        setSigningOut(true);
      },
    });
  }, [runGated, publishStatus]);

  const activateThisDevice = useCallback(
    (confirmRevokeOldest: boolean) => {
      if (!isTauri() || busyRef.current) return;
      runGated(() => invokeCommand('activate_this_device', { confirmRevokeOldest }), publishStatus, {
        onStart: () => {
          setSignInNotice(null);
          setActivating(true);
        },
      });
    },
    [runGated, publishStatus],
  );

  const deleteAccount = useCallback(() => {
    if (!isTauri()) return;
    runGated(
      () => invokeCommand('delete_account', {}),
      (report) => {
        setDeleteReport(report);
        publishStatus(report.license);
      },
      {
        onStart: () => {
          setDeleteReport(null);
          setDeleteError(null);
          setDeleting(true);
        },
        onError: (failure) => setDeleteError(formatNativeError(failure)),
      },
    );
  }, [runGated, publishStatus]);

  const dismissDeleteResult = useCallback(() => {
    setDeleteReport(null);
    setDeleteError(null);
  }, []);

  const startCheckout = useCallback(
    (seats: number) => {
      if (!isTauri()) return;
      const token = checkoutGate.begin();
      setCheckout({ busy: true, error: null, url: null });
      void invokeCommand('start_checkout', { quantity: seats })
        .then((session) => {
          if (checkoutGate.isCurrent(token)) setCheckout({ busy: false, error: null, url: session.url });
        })
        .catch((failure: unknown) => {
          if (checkoutGate.isCurrent(token)) setCheckout({ busy: false, error: formatNativeError(failure), url: null });
        });
    },
    [checkoutGate],
  );

  const controller = useMemo<LicenseController>(
    () => ({
      status,
      error,
      refreshing,
      signInProvider,
      signInNotice,
      signingOut,
      activating,
      deleting,
      deleteReport,
      deleteError,
      checkout,
      refresh,
      signIn,
      signOut,
      activateThisDevice,
      deleteAccount,
      dismissDeleteResult,
      startCheckout,
    }),
    [status, error, refreshing, signInProvider, signInNotice, signingOut, activating, deleting, deleteReport, deleteError, checkout, refresh, signIn, signOut, activateThisDevice, deleteAccount, dismissDeleteResult, startCheckout],
  );

  return <LicenseContext.Provider value={controller}>{children}</LicenseContext.Provider>;
}
