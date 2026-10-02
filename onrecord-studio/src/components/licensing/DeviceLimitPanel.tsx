import { useEffect, useRef, useState } from 'react';

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
    <div className="mt-6 space-y-3" role="group" aria-label="Device limit options">
      {confirming ? (
        <div className="rounded-lg border border-warning bg-warning/40 px-4 py-3 text-sm leading-6 text-warning">
          <p ref={confirmRef} tabIndex={-1} className="font-semibold outline-none">
            Sign out your oldest Mac and use this one?
          </p>
          <p className="mt-1 text-label leading-5">
            The Mac that signed in longest ago will be signed out of this account and will need to sign in again. Local
            evidence on that Mac is not deleted.
          </p>
          <div className="mt-3 flex flex-wrap gap-2">
            <button
              type="button"
              disabled={busy}
              aria-busy={activating}
              onClick={() => onActivate(true)}
              className="focus-ring inline-flex min-h-[24px] items-center rounded-lg bg-warning px-3 py-2 text-sm font-semibold text-muted hover:bg-warning disabled:cursor-wait disabled:opacity-50"
            >
              {activating ? 'Signing out oldest Mac...' : 'Yes, sign out my oldest Mac'}
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => setConfirming(false)}
              className="focus-ring inline-flex min-h-[24px] items-center rounded-lg border border-control px-3 py-2 text-sm font-semibold text-fg hover:bg-raised disabled:opacity-50"
            >
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <button
          type="button"
          disabled={busy}
          onClick={() => setConfirming(true)}
          className="focus-ring inline-flex min-h-[24px] w-full items-center justify-center rounded-lg bg-accent px-4 py-3 text-sm font-semibold text-muted hover:bg-accent disabled:opacity-50"
        >
          Use this Mac and sign out my oldest Mac
        </button>
      )}
      <button
        type="button"
        disabled={busy}
        onClick={onSignOut}
        className="focus-ring inline-flex min-h-[24px] w-full items-center justify-center rounded-lg border border-control px-4 py-3 text-sm font-semibold text-fg hover:bg-raised disabled:opacity-50"
      >
        Sign out
      </button>
    </div>
  );
}
