import { useEffect, useState } from 'react';
import Button from '../ui/Button';
import Dialog, { DialogBody, DialogFooter, DialogHeader } from '../ui/Dialog';
import { Input } from '../ui/Field';
import Skeleton from '../ui/Skeleton';
import { formatNativeError, invokeCommand } from '../../ipc';
import { useLatestRequestGate } from '../../hooks/useLatestRequestGate';
import type { SeatEntry, SeatStatus } from '../../types';
import { formatShortDate } from '../objects/labels';

function seatState(seat: SeatEntry): string {
  if (seat.revokedAt) return `Revoked ${formatShortDate(seat.revokedAt)}`;
  if (seat.claimedAt) return `In use since ${formatShortDate(seat.claimedAt)}`;
  return `Invited ${formatShortDate(seat.invitedAt)}, not yet signed in`;
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
    <Dialog onClose={onClose}>
      <DialogHeader title="Seats" />
      <DialogBody>
      {status && !status.subscriptionActive ? (
        <p className="text-body text-fg-secondary">
          Seats are managed by the purchaser of an active subscription. If a colleague bought the seats, ask them to assign yours; if your subscription lapsed, renew it under Manage subscription.
        </p>
      ) : null}
      {status?.subscriptionActive ? (
        <>
          <p className="text-body tabular-nums text-fg-secondary">
            {status.quantity} {status.quantity === 1 ? 'seat' : 'seats'} purchased: yours, plus {status.usedSeats} assigned and {remaining} free.
          </p>
          <p className="mt-1 text-caption text-muted">
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
            <Input
              id="seat-email"
              type="email"
              required
              value={email}
              disabled={busy || remaining === 0}
              onChange={(event) => setEmail(event.target.value)}
              placeholder="name@example.org"
              className="min-w-0 flex-1"
            />
            <Button type="submit" variant="primary" disabled={busy || remaining === 0 || email.trim() === ''}>
              Assign seat
            </Button>
          </form>
          {remaining === 0 ? (
            <p className="mt-2 text-caption text-muted">Every purchased seat is assigned. Revoke one, or add seats under Manage subscription.</p>
          ) : null}
          <ul className="mt-4 divide-y divide-line text-body">
            {active.length === 0 ? <li className="py-2 text-muted">No seats assigned yet.</li> : null}
            {active.map((seat) => (
              <li key={seat.email} className="flex items-center justify-between gap-3 py-2">
                <div className="min-w-0">
                  <div className="truncate text-fg">{seat.email}</div>
                  <div className="text-caption text-muted">{seatState(seat)}</div>
                </div>
                <Button variant="danger" size="sm" disabled={busy} onClick={() => void run('revoke_seat', { email: seat.email })}>
                  Revoke
                </Button>
              </li>
            ))}
          </ul>
          {revoked.length > 0 ? (
            <details className="mt-3 text-caption text-muted">
              <summary>{revoked.length} revoked</summary>
              <ul className="mt-1 space-y-1">
                {revoked.map((seat) => <li key={seat.email}>{seat.email}: {seatState(seat)}. Assign it again to restore access.</li>)}
              </ul>
            </details>
          ) : null}
        </>
      ) : null}
      {!status && !error ? <Skeleton variant="lines" lines={3} label="Loading seats" /> : null}
      {error ? <p role="alert" className="mt-3 text-body text-danger">{error}</p> : null}
      </DialogBody>
      <DialogFooter>
        <Button onClick={onClose}>Done</Button>
      </DialogFooter>
    </Dialog>
  );
}
