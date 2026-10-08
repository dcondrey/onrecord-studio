import { useId, useState } from 'react';
import { useLicense } from '../licensing/LicenseProvider';
import { providerLabel } from '../licensing/providers';
import Button from '../ui/Button';
import Dialog, { DialogBody, DialogHeader } from '../ui/Dialog';
import Icon from '../ui/icons';
import { useAppStore } from '../../store/useAppStore';
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
    return ends ? `Trial · ends ${ends}` : 'Trial';
  }
  if (status.state === 'LICENSED') {
    const until = status.entitlementExpiresAt && formatShortDate(status.entitlementExpiresAt);
    return until ? `Licensed · renews ${until}` : 'Licensed';
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
  const audioAlertsEnabled = useAppStore((state) => state.audioAlertsEnabled);
  const setAudioAlertsEnabled = useAppStore((state) => state.setAudioAlertsEnabled);
  const billingHintId = useId();
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

  const initial = (status.accountEmail ?? '').trim().charAt(0).toUpperCase();
  const licensed = status.state === 'LICENSED';

  return (
    <>
      <button
        type="button"
        aria-label="Account"
        title="Account"
        onClick={() => setOpen(true)}
        className="focus-ring grid h-8 w-8 shrink-0 place-items-center rounded-full bg-raised text-control text-fg-secondary transition-colors hover:bg-line hover:text-fg"
      >
        {initial ? <span aria-hidden="true">{initial}</span> : <Icon name="globe" size="sm" />}
      </button>
      {open ? (
        <Dialog onClose={close}>
          <DialogHeader title="Account" />
          <DialogBody>
            <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-6 gap-y-2 text-body">
              <dt className="text-muted">Email</dt>
              <dd className="min-w-0 break-words text-fg">{status.accountEmail ?? 'Not reported'}</dd>
              <dt className="text-muted">Signed in with</dt>
              <dd className="text-fg">{status.signInProvider ? providerLabel(status.signInProvider) : 'Not reported'}</dd>
              <dt className="text-muted">Plan</dt>
              <dd className="tabular-nums text-fg">{planLine(status)}</dd>
            </dl>
            <div className="mt-5 flex flex-wrap gap-2">
              <Button
                onClick={() => {
                  close();
                  setProfileOpen(true);
                }}
              >
                Profile settings
              </Button>
              {licensed ? (
                <Button disabled={busy || billingBusy} aria-describedby={billingHintId} onClick={() => void manageSubscription()} className="peer">
                  {billingBusy ? 'Opening Stripe…' : 'Manage subscription'}
                </Button>
              ) : null}
              {licensed ? (
                <Button
                  disabled={busy}
                  onClick={() => {
                    close();
                    setSeatsOpen(true);
                  }}
                >
                  Seats
                </Button>
              ) : null}
              {licensed ? (
                <p id={billingHintId} className="hidden basis-full text-caption text-muted peer-hover:block peer-focus-visible:block">
                  Opens Stripe in your browser to change seats or payment method, or to cancel. Cancelling stops renewal at the end of the current billing period; the license stays active until then and no partial refund is issued.
                </p>
              ) : null}
            </div>
            <label className="mt-5 flex items-center justify-between gap-4 border-t border-line pt-4 text-body text-fg">
              <span>
                Audio alerts
                <span className="block text-caption text-muted">A sound when a source finishes syncing or a task fails.</span>
              </span>
              <input
                type="checkbox"
                role="switch"
                checked={audioAlertsEnabled}
                aria-checked={audioAlertsEnabled}
                onChange={(event) => setAudioAlertsEnabled(event.currentTarget.checked)}
                className="focus-ring"
              />
            </label>
            {billingNotice ? <p role="alert" className="mt-3 text-caption text-warning">{billingNotice}</p> : null}
            {confirming ? (
              <div role="group" aria-label="Confirm account deletion" className="mt-4 rounded-lg border border-danger/30 bg-danger/10 p-4 text-body text-danger">
                <p className="font-semibold">Delete your account?</p>
                <ul className="mt-2 list-disc space-y-1 pl-4">
                  <li>Your license account is deleted on the server.</li>
                  <li>Local sign-in data on this Mac is removed.</li>
                  <li>Local evidence on this Mac is not deleted.</li>
                  <li>A trial already used on this device is not restored.</li>
                </ul>
                <div className="mt-3 flex gap-2">
                  <Button
                    variant="destructive"
                    disabled={controller.deleting}
                    onClick={() => {
                      setConfirming(false);
                      controller.deleteAccount();
                    }}
                  >
                    Delete my account
                  </Button>
                  <Button onClick={() => setConfirming(false)}>Cancel</Button>
                </div>
              </div>
            ) : null}
            {controller.deleting ? <p role="status" className="mt-3 text-caption">Deleting account…</p> : null}
            {controller.error ? <p role="alert" className="mt-3 text-caption text-danger">{controller.error}</p> : null}
            <DeleteAccountResult report={controller.deleteReport} error={controller.deleteError} onDismiss={controller.dismissDeleteResult} />
          </DialogBody>
          <div className="flex shrink-0 items-center gap-2 border-t border-line px-5 py-4">
            <Button variant="ghost" disabled={busy} onClick={() => setConfirming(true)} className="-ml-3 text-danger hover:bg-danger/10 hover:text-danger">
              Delete account…
            </Button>
            <Button disabled={busy} onClick={controller.signOut} className="ml-auto">
              {controller.signingOut ? 'Signing out…' : 'Sign out'}
            </Button>
          </div>
        </Dialog>
      ) : null}
      {seatsOpen ? <SeatsPanel onClose={() => setSeatsOpen(false)} /> : null}
    </>
  );
}
