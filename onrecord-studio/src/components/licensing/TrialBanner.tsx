import { useState } from 'react';
import { daysLeftLabel, trialDaysLeft } from '../../utils/licenseState';
import { cn } from '../../utils/cn';
import Button from '../ui/Button';
import Dialog, { DialogBody, DialogHeader } from '../ui/Dialog';
import Icon from '../ui/icons';
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
        className={cn(
          'flex h-10 items-center gap-2 border-b px-4 text-body',
          urgent ? 'border-warning/40 bg-warning/10 text-warning' : 'border-line bg-surface text-fg-secondary',
        )}
      >
        <Icon name="clock" size="sm" />
        <p className={cn('tabular-nums', urgent && 'font-semibold')}>
          {urgent ? 'Trial ending soon · ' : 'Free trial · '}
          {days === null ? 'active' : daysLeftLabel(days)}
        </p>
        <Button variant="primary" size="sm" onClick={() => setUpgradeOpen(true)} className="ml-auto">
          Upgrade
        </Button>
        <Button variant="ghost" size="sm" iconOnly icon="x" aria-label="Dismiss trial notice for this session" onClick={dismiss} />
      </div>
      {upgradeOpen ? (
        <Dialog onClose={() => setUpgradeOpen(false)}>
          <DialogHeader title="Upgrade OnRecord Studio" />
          <DialogBody className="[&>section]:mt-0">
            <UpgradePanel checkout={controller.checkout} onStartCheckout={controller.startCheckout} />
          </DialogBody>
        </Dialog>
      ) : null}
    </>
  );
}
