import { useState } from 'react';
import ModalDialog from '../licensing/ModalDialog';
import { useLicense } from '../licensing/LicenseProvider';
import { useProfileLensStore } from '../../store/useProfileLensStore';
import { isDirectChannel } from '../../utils/licenseState';
import type { LicenseStatus } from '../../types';
import DeleteAccountResult from './DeleteAccountResult';

function formatDate(value: string | null): string | null {
  if (!value) return null;
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleDateString(undefined, { dateStyle: 'medium' });
}

function planLine(status: LicenseStatus): string {
  if (status.state === 'TRIAL') {
    const ends = formatDate(status.trialEndsAt);
    return ends ? `Free trial, ends ${ends}` : 'Free trial';
  }
  if (status.state === 'LICENSED') {
    const until = formatDate(status.entitlementExpiresAt);
    return until ? `Licensed, verified license valid until ${until}` : 'Licensed';
  }
  return 'Plan not reported';
}

/** Rendered only for the direct build, where an account exists. */
export default function AccountMenu() {
  const controller = useLicense();
  const setProfileOpen = useProfileLensStore((state) => state.setSettingsOpen);
  const [open, setOpen] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const status = controller?.status ?? null;
  if (!controller || !status || !isDirectChannel(status)) return null;

  const busy = controller.signingOut || controller.deleting;
  const hasResult = controller.deleteReport !== null || controller.deleteError !== null;
  const close = () => {
    setOpen(false);
    setConfirming(false);
    if (hasResult) controller.dismissDeleteResult();
  };

  return (
    <>
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="focus-ring h-9 rounded-xl border border-slate-800 bg-slate-900/50 px-3 text-xs font-medium text-slate-300 transition hover:text-slate-100"
      >
        Account
      </button>
      {open ? (
        <ModalDialog label="Account" onClose={close}>
          <h2 className="text-lg font-semibold">Account</h2>
          <dl className="mt-3 space-y-1 text-sm text-slate-300">
            <div className="flex gap-2"><dt className="text-slate-400">Email</dt><dd>{status.accountEmail ?? 'Not reported'}</dd></div>
            <div className="flex gap-2"><dt className="text-slate-400">Sign-in</dt><dd>{status.signInProvider ?? 'Not reported'}</dd></div>
            <div className="flex gap-2"><dt className="text-slate-400">Plan</dt><dd>{planLine(status)}</dd></div>
          </dl>
          <div className="mt-4 flex flex-wrap gap-2">
            <button
              type="button"
              onClick={() => {
                close();
                setProfileOpen(true);
              }}
              className="focus-ring rounded-lg border border-slate-500 px-3 py-1.5 text-xs font-semibold hover:bg-slate-800"
            >
              Profile settings
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={controller.signOut}
              className="focus-ring rounded-lg border border-slate-500 px-3 py-1.5 text-xs font-semibold hover:bg-slate-800 disabled:opacity-50"
            >
              {controller.signingOut ? 'Signing out...' : 'Sign out'}
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => setConfirming(true)}
              className="focus-ring rounded-lg border border-rose-600 px-3 py-1.5 text-xs font-semibold text-rose-200 hover:bg-rose-950 disabled:opacity-50"
            >
              Delete account
            </button>
          </div>
          {confirming ? (
            <div role="group" aria-label="Confirm account deletion" className="mt-4 rounded-lg border border-rose-700 bg-rose-950/60 p-4 text-xs leading-5 text-rose-50">
              <p className="font-semibold">Delete your account?</p>
              <ul className="mt-2 list-disc space-y-1 pl-4">
                <li>Your license account is deleted on the server.</li>
                <li>Local sign-in data on this Mac is removed.</li>
                <li>Local evidence on this Mac is not deleted.</li>
                <li>A trial already used on this device is not restored.</li>
              </ul>
              <div className="mt-3 flex gap-2">
                <button
                  type="button"
                  disabled={controller.deleting}
                  onClick={() => {
                    setConfirming(false);
                    controller.deleteAccount();
                  }}
                  className="focus-ring rounded-lg bg-rose-600 px-3 py-1.5 font-semibold text-white hover:bg-rose-500 disabled:opacity-50"
                >
                  Delete my account
                </button>
                <button
                  type="button"
                  onClick={() => setConfirming(false)}
                  className="focus-ring rounded-lg border border-slate-400 px-3 py-1.5 font-semibold hover:bg-slate-800"
                >
                  Cancel
                </button>
              </div>
            </div>
          ) : null}
          {controller.deleting ? <p role="status" className="mt-3 text-xs">Deleting account...</p> : null}
          {controller.error ? <p role="alert" className="mt-3 text-xs text-rose-200">{controller.error}</p> : null}
          <DeleteAccountResult report={controller.deleteReport} error={controller.deleteError} onDismiss={controller.dismissDeleteResult} />
        </ModalDialog>
      ) : null}
    </>
  );
}
