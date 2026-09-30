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
        <div className="rounded-lg border border-amber-600 bg-amber-950/50 px-4 py-3 text-sm leading-6 text-amber-50">
          <p ref={confirmRef} tabIndex={-1} className="font-semibold outline-none">
            Sign out your oldest Mac and use this one?
          </p>
          <p className="mt-1 text-xs leading-5">
            The Mac that signed in longest ago will be signed out of this account and will need to sign in again. Local
            evidence on that Mac is not deleted.
          </p>
          <div className="mt-3 flex flex-wrap gap-2">
            <button
              type="button"
              disabled={busy}
              aria-busy={activating}
              onClick={() => onActivate(true)}
              className="focus-ring inline-flex min-h-[24px] items-center rounded-lg bg-amber-300 px-3 py-2 text-sm font-semibold text-slate-950 hover:bg-amber-200 disabled:cursor-wait disabled:opacity-50"
            >
              {activating ? 'Signing out oldest Mac...' : 'Yes, sign out my oldest Mac'}
            </button>
            <button
              type="button"
              disabled={busy}
              onClick={() => setConfirming(false)}
              className="focus-ring inline-flex min-h-[24px] items-center rounded-lg border border-slate-500 px-3 py-2 text-sm font-semibold text-slate-100 hover:bg-slate-800 disabled:opacity-50"
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
          className="focus-ring inline-flex min-h-[24px] w-full items-center justify-center rounded-lg bg-cyan-500 px-4 py-3 text-sm font-semibold text-slate-950 hover:bg-cyan-400 disabled:opacity-50"
        >
          Use this Mac and sign out my oldest Mac
        </button>
      )}
      <button
        type="button"
        disabled={busy}
        onClick={onSignOut}
        className="focus-ring inline-flex min-h-[24px] w-full items-center justify-center rounded-lg border border-slate-500 px-4 py-3 text-sm font-semibold text-slate-100 hover:bg-slate-800 disabled:opacity-50"
      >
        Sign out
      </button>
    </div>
  );
}
