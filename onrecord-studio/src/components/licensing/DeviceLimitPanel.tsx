import { useEffect, useRef, useState } from 'react';
import Button from '../ui/Button';

interface Props {
  busy: boolean;
  activating: boolean;
  onActivate: (confirmRevokeOldest: boolean) => void;
  onSignOut: () => void;
}

/** The revoking call is reachable only through the second, explicit confirmation. */
export default function DeviceLimitPanel({ busy, activating, onActivate, onSignOut }: Props) {
  const [confirming, setConfirming] = useState(false);
  const confirmRef = useRef<HTMLParagraphElement>(null);

  useEffect(() => {
    if (confirming) confirmRef.current?.focus();
  }, [confirming]);

  return (
    <div className="mt-6 flex flex-col gap-2" role="group" aria-label="Device limit options">
      {confirming ? (
        <div className="rounded-lg border border-warning/30 bg-warning/10 px-4 py-3 text-body text-warning">
          <p ref={confirmRef} tabIndex={-1} className="font-semibold outline-none">
            Sign out your oldest Mac and use this one?
          </p>
          <p className="mt-1 text-caption">
            The Mac that signed in longest ago will be signed out of this account and will need to sign in again. Local
            evidence on that Mac is not deleted.
          </p>
          <div className="mt-3 flex flex-wrap gap-2">
            <Button variant="primary" disabled={busy} aria-busy={activating} onClick={() => onActivate(true)}>
              {activating ? 'Signing out oldest Mac…' : 'Yes, sign out my oldest Mac'}
            </Button>
            <Button disabled={busy} onClick={() => setConfirming(false)}>
              Cancel
            </Button>
          </div>
        </div>
      ) : (
        <Button variant="primary" size="lg" disabled={busy} onClick={() => setConfirming(true)} className="w-full">
          Use this Mac and sign out my oldest Mac
        </Button>
      )}
      <Button size="lg" disabled={busy} onClick={onSignOut} className="w-full">
        Sign out
      </Button>
    </div>
  );
}
