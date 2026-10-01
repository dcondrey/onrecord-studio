import { useEffect, useState } from 'react';
import ModalDialog from '../licensing/ModalDialog';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { SeatEntry, SeatStatus } from '../../types';

function when(value: string | null): string {
  if (!value) return '';
  const parsed = new Date(value);
  return Number.isNaN(parsed.getTime()) ? value : parsed.toLocaleDateString(undefined, { dateStyle: 'medium' });
}

function seatState(seat: SeatEntry): string {
  if (seat.revokedAt) return `Revoked ${when(seat.revokedAt)}`;
  if (seat.claimedAt) return `In use since ${when(seat.claimedAt)}`;
  return `Invited ${when(seat.invitedAt)}, not yet signed in`;
}

/** Seat assignment for the purchaser of an organization subscription. Every change re-reads the
 * list from the server, so what is shown is what the server holds, never a local guess. */
export default function SeatsPanel({ onClose }: { onClose: () => void }) {
  const gate = useLatestRequestGate();
  const [status, setStatus] = useState<SeatStatus | null>(null);
  const [email, setEmail] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const run = async (command: 'list_seats' | 'invite_seat' | 'revoke_seat', args: { email?: string } = {}) => {
    const token = gate.begin();
    setBusy(true);
    setError(null);
    try {
      const next = await (command === 'list_seats'
        ? invokeCommand('list_seats', {})
        : invokeCommand(command, { email: args.email ?? '' }));
      if (!gate.isCurrent(token)) return;
      setStatus(next);
      if (command === 'invite_seat') setEmail('');
    } catch (failure) {
      if (gate.isCurrent(token)) setError(formatNativeError(failure));
    } finally {
      if (gate.isCurrent(token)) setBusy(false);
    }
  };

  useEffect(() => {
    void run('list_seats');
  }, []);

  const active = status?.seats.filter((seat) => !seat.revokedAt) ?? [];
  const revoked = status?.seats.filter((seat) => seat.revokedAt) ?? [];
  const remaining = status ? Math.max(0, status.quantity - 1 - status.usedSeats) : 0;

  return (
    <ModalDialog label="Seats" onClose={onClose}>
      <h2 className="text-lg font-semibold">Seats</h2>
      {status && !status.subscriptionActive ? (
        <p className="mt-3 text-sm text-slate-300">
          Seats are managed by the purchaser of an active subscription. If a colleague bought the seats, ask them to assign yours; if your subscription lapsed, renew it under Manage subscription.
        </p>
      ) : null}
      {status?.subscriptionActive ? (
        <>
          <p className="mt-3 text-sm text-slate-300">
            {status.quantity} {status.quantity === 1 ? 'seat' : 'seats'} purchased: yours, plus {status.usedSeats} assigned and {remaining} free.
          </p>
          <p className="mt-1 text-xs leading-5 text-slate-400">
            No email is sent. Tell your colleague to sign in to OnRecord Studio with the address you enter; the seat is theirs at their first license check, once their sign-in provider has confirmed the address. Revoking locks their workspace at their next check. Their local evidence is never deleted.
          </p>
          <form
            className="mt-3 flex flex-wrap gap-2"
            onSubmit={(event) => {
              event.preventDefault();
              void run('invite_seat', { email });
            }}
          >
            <label className="sr-only" htmlFor="seat-email">Colleague's email address</label>
            <input
              id="seat-email"
              type="email"
              required
              value={email}
              disabled={busy || remaining === 0}
              onChange={(event) => setEmail(event.target.value)}
              placeholder="name@example.org"
              className="focus-ring h-9 min-w-0 flex-1 rounded-lg border border-slate-700 bg-slate-950/60 px-3 text-sm text-slate-100 placeholder:text-slate-400"
            />
            <button
              type="submit"
              disabled={busy || remaining === 0 || email.trim() === ''}
              className="focus-ring rounded-lg border border-slate-500 px-3 py-1.5 text-xs font-semibold hover:bg-slate-800 disabled:opacity-50"
            >
              Assign seat
            </button>
          </form>
          {remaining === 0 ? (
            <p className="mt-2 text-xs text-slate-400">Every purchased seat is assigned. Revoke one, or add seats under Manage subscription.</p>
          ) : null}
          <ul className="mt-4 divide-y divide-slate-800 text-sm">
            {active.length === 0 ? <li className="py-2 text-slate-400">No seats assigned yet.</li> : null}
            {active.map((seat) => (
              <li key={seat.email} className="flex items-center justify-between gap-3 py-2">
                <div className="min-w-0">
                  <div className="truncate text-slate-100">{seat.email}</div>
                  <div className="text-xs text-slate-400">{seatState(seat)}</div>
                </div>
                <button
                  type="button"
                  disabled={busy}
                  onClick={() => void run('revoke_seat', { email: seat.email })}
                  className="focus-ring shrink-0 rounded-lg border border-rose-600 px-2.5 py-1 text-xs font-semibold text-rose-200 hover:bg-rose-950 disabled:opacity-50"
                >
                  Revoke
                </button>
              </li>
            ))}
          </ul>
          {revoked.length > 0 ? (
            <details className="mt-3 text-xs text-slate-400">
              <summary className="cursor-pointer">{revoked.length} revoked</summary>
              <ul className="mt-1 space-y-1">
                {revoked.map((seat) => <li key={seat.email}>{seat.email}: {seatState(seat)}. Assign it again to restore access.</li>)}
              </ul>
            </details>
          ) : null}
        </>
      ) : null}
      {!status && !error ? <p className="mt-3 text-sm text-slate-400">Loading seats...</p> : null}
      {error ? <p role="alert" className="mt-3 text-sm text-amber-200">{error}</p> : null}
      <div className="mt-4 flex justify-end">
        <button type="button" onClick={onClose} className="focus-ring rounded-lg border border-slate-500 px-3 py-1.5 text-xs font-semibold hover:bg-slate-800">Close</button>
      </div>
    </ModalDialog>
  );
}
