import { useState } from 'react';
import ModalDialog from '../licensing/ModalDialog';
import Button from '../ui/Button';
import { useLicense } from '../licensing/LicenseProvider';
import { useProfileLensStore } from '../../store/useProfileLensStore';
import { isDirectChannel } from '../../utils/licenseState';
import type { LicenseStatus } from '../../types';
import DeleteAccountResult from './DeleteAccountResult';
import SeatsPanel from './SeatsPanel';
import { formatNativeError, invokeCommand } from '../../ipc';
import { openExternalUrl } from '../../utils/openExternal';
import { formatShortDate } from '../objects/labels';

function planLine(status: LicenseStatus): string {
  if (status.state === 'TRIAL') {
    const ends = status.trialEndsAt && formatShortDate(status.trialEndsAt);
    return ends ? `Free trial, ends ${ends}` : 'Free trial';
  }
  if (status.state === 'LICENSED') {
    const until = status.entitlementExpiresAt && formatShortDate(status.entitlementExpiresAt);
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
  const [billingBusy, setBillingBusy] = useState(false);
  const [billingNotice, setBillingNotice] = useState<string | null>(null);
  const [seatsOpen, setSeatsOpen] = useState(false);
  const status = controller?.status ?? null;
  if (!controller || !status || !isDirectChannel(status)) return null;

  // Stripe's Customer Portal owns payment method, seat count and cancellation. Cancelling there
  // ends renewal at the period end; the entitlement stays valid until then and nothing is refunded.
  const manageSubscription = async () => {
    setBillingBusy(true);
    setBillingNotice(null);
    try {
      const session = await invokeCommand('start_billing_portal', {});
      const opened = await openExternalUrl(session.url);
      if (!opened.ok) setBillingNotice(`${opened.reason} Address: ${session.url}`);
    } catch (failure) {
      setBillingNotice(formatNativeError(failure));
    } finally {
      setBillingBusy(false);
    }
  };

  const busy = controller.signingOut || controller.deleting;
  const hasResult = controller.deleteReport !== null || controller.deleteError !== null;
  const close = () => {
    setOpen(false);
    setConfirming(false);
    if (hasResult) controller.dismissDeleteResult();
  };

  return (
    <>
      <Button variant="ghost" iconOnly icon="user" title="Account" aria-label="Account" onClick={() => setOpen(true)} />
      {open ? (
        <ModalDialog label="Account" onClose={close}>
          <h2 className="text-lg font-semibold">Account</h2>
          <dl className="mt-3 space-y-1 text-sm text-fg-secondary">
            <div className="flex gap-2"><dt className="text-muted">Email</dt><dd>{status.accountEmail ?? 'Not reported'}</dd></div>
            <div className="flex gap-2"><dt className="text-muted">Sign-in</dt><dd>{status.signInProvider ?? 'Not reported'}</dd></div>
            <div className="flex gap-2"><dt className="text-muted">Plan</dt><dd>{planLine(status)}</dd></div>
          </dl>
          <div className="mt-4 flex flex-wrap gap-2">
            <button
              type="button"
              onClick={() => {
                close();
                setProfileOpen(true);
              }}
              className="focus-ring rounded-lg border border-control px-3 py-1.5 text-label leading-4 font-semibold hover:bg-raised"
            >
              Profile settings
            </button>
            {status.state === 'LICENSED' ? (
              <button
                type="button"
                disabled={busy || billingBusy}
                onClick={() => void manageSubscription()}
                className="focus-ring rounded-lg border border-control px-3 py-1.5 text-label leading-4 font-semibold hover:bg-raised disabled:opacity-50"
              >
                {billingBusy ? 'Opening Stripe...' : 'Manage subscription'}
              </button>
            ) : null}
            {status.state === 'LICENSED' ? (
              <button
                type="button"
                disabled={busy}
                onClick={() => {
                  close();
                  setSeatsOpen(true);
                }}
                className="focus-ring rounded-lg border border-control px-3 py-1.5 text-label leading-4 font-semibold hover:bg-raised disabled:opacity-50"
              >
                Seats
              </button>
            ) : null}
            <button
              type="button"
              disabled={busy}
              onClick={controller.signOut}
              className="focus-ring rounded-lg border border-control px-3 py-1.5 text-label leading-4 font-semibold hover:bg-raised disabled:opacity-50"
            >
              {controller.signingOut ? 'Signing out...' : 'Sign out'}
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => setConfirming(true)}
              className="focus-ring rounded-lg border border-danger px-3 py-1.5 text-label leading-4 font-semibold text-danger hover:bg-danger/10 disabled:opacity-50"
            >
              Delete account
            </button>
          </div>
          {status.state === 'LICENSED' ? (
            <p className="mt-2 text-label leading-5 text-muted">
              Manage subscription opens Stripe in your browser to change seats or payment method, or to cancel. Cancelling stops renewal at the end of the current billing period; the license stays active until then and no partial refund is issued.
            </p>
          ) : null}
          {billingNotice ? <p role="alert" className="mt-2 text-label leading-4 text-warning">{billingNotice}</p> : null}
          {confirming ? (
            <div role="group" aria-label="Confirm account deletion" className="mt-4 rounded-lg border border-danger/30 bg-danger/10 p-4 text-label leading-5 text-danger">
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
                  className="focus-ring rounded-lg bg-danger px-3 py-1.5 font-semibold text-white hover:bg-danger disabled:opacity-50"
                >
                  Delete my account
                </button>
                <button
                  type="button"
                  onClick={() => setConfirming(false)}
                  className="focus-ring rounded-lg border border-control px-3 py-1.5 font-semibold hover:bg-raised"
                >
                  Cancel
                </button>
              </div>
            </div>
          ) : null}
          {controller.deleting ? <p role="status" className="mt-3 text-label leading-4">Deleting account...</p> : null}
          {controller.error ? <p role="alert" className="mt-3 text-label leading-4 text-danger">{controller.error}</p> : null}
          <DeleteAccountResult report={controller.deleteReport} error={controller.deleteError} onDismiss={controller.dismissDeleteResult} />
        </ModalDialog>
      ) : null}
      {seatsOpen ? <SeatsPanel onClose={() => setSeatsOpen(false)} /> : null}
    </>
  );
}
