import { useState } from 'react';
import { daysLeftLabel, trialDaysLeft } from '../../utils/licenseState';
import ModalDialog from './ModalDialog';
import { useLicense } from './LicenseProvider';
import UpgradePanel from './UpgradePanel';

const URGENT_DAYS = 3;
const DISMISS_KEY = 'onrecord.trialBanner.dismissed';

function readDismissed(): string | null {
  try {
    return window.sessionStorage.getItem(DISMISS_KEY);
  } catch {
    return null;
  }
}

/** Slim, never blocking. Shown only while the state is TRIAL. */
export default function TrialBanner() {
  const controller = useLicense();
  const [dismissedLevel, setDismissedLevel] = useState<string | null>(readDismissed);
  const [upgradeOpen, setUpgradeOpen] = useState(false);
  const status = controller?.status;
  if (!controller || !status || status.state !== 'TRIAL' || !status.authorized) return null;

  const days = trialDaysLeft(status);
  const urgent = days !== null && days <= URGENT_DAYS;
  const level = urgent ? 'urgent' : 'normal';
  // Dismissal is per urgency level, so a banner hidden at 20 days returns for the final 3.
  if (dismissedLevel === level && !upgradeOpen) return null;

  const dismiss = () => {
    setDismissedLevel(level);
    try {
      window.sessionStorage.setItem(DISMISS_KEY, level);
    } catch {
      // The banner is only hidden for this mount when storage is unavailable.
    }
  };

  return (
    <>
      <div
        role="status"
        className={
          urgent
            ? 'flex items-center gap-3 border-b border-warning bg-warning px-4 py-1.5 text-label leading-4 text-warning'
            : 'flex items-center gap-3 border-b border-line-strong bg-surface px-4 py-1.5 text-label leading-4 text-fg'
        }
      >
        <span aria-hidden="true">{urgent ? '▲' : '●'}</span>
        <p className={urgent ? 'font-semibold' : ''}>
          {urgent ? 'Trial ending soon: ' : 'Free trial: '}
          {days === null ? 'trial active' : daysLeftLabel(days)}
        </p>
        <button
          type="button"
          onClick={() => setUpgradeOpen(true)}
          className="focus-ring ml-auto rounded border border-current px-2 py-1 font-semibold hover:underline"
        >
          Upgrade
        </button>
        <button
          type="button"
          aria-label="Dismiss trial notice for this session"
          onClick={dismiss}
          className="focus-ring rounded px-1.5 py-1 font-semibold hover:underline"
        >
          Dismiss
        </button>
      </div>
      {upgradeOpen ? (
        <ModalDialog label="Upgrade OnRecord Studio" onClose={() => setUpgradeOpen(false)}>
          <UpgradePanel checkout={controller.checkout} onStartCheckout={controller.startCheckout} />
        </ModalDialog>
      ) : null}
    </>
  );
}
