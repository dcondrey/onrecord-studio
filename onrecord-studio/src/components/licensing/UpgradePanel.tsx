import { useEffect, useId, useRef, useState } from 'react';
import {
  MAX_SEATS,
  MIN_SEATS,
  PRICE_TERMS,
  PRICE_TIERS,
  clampSeats,
  tierForSeats,
  tierLabel,
  yearlyTotalUsd,
} from '../../config/pricing';
import { openExternalUrl } from '../../utils/openExternal';
import type { CheckoutState } from './LicenseProvider';

/** Price facts, rendered only from `config/pricing.ts`. */
export function PriceTable() {
  return (
    <div>
      <table className="w-full text-left text-label leading-4 text-fg-secondary">
        <caption className="sr-only">Price per seat per year</caption>
        <thead>
          <tr className="text-muted">
            <th scope="col" className="py-1 pr-3 font-medium">Seats</th>
            <th scope="col" className="py-1 font-medium">Per seat, per year</th>
          </tr>
        </thead>
        <tbody>
          {PRICE_TIERS.map((tier) => (
            <tr key={tier.minSeats}>
              <td className="py-0.5 pr-3">{tierLabel(tier)}</td>
              <td className="py-0.5 tabular-nums">${tier.usdPerSeatPerYear}</td>
            </tr>
          ))}
        </tbody>
      </table>
      <p className="mt-2 text-label leading-4 text-muted">{PRICE_TERMS}</p>
    </div>
  );
}

export default function UpgradePanel({
  checkout,
  onStartCheckout,
}: {
  checkout: CheckoutState;
  onStartCheckout: (seats: number) => void;
}) {
  const [seats, setSeats] = useState(MIN_SEATS);
  const [openError, setOpenError] = useState<string | null>(null);
  const seatsId = useId();
  const tier = tierForSeats(seats);

  const latestUrl = useRef<string | null>(null);
  const openCheckout = (url: string) => {
    latestUrl.current = url;
    void openExternalUrl(url).then((result) => {
      // A newer checkout replaced this one while the browser was opening; only the latest reports.
      if (latestUrl.current === url) setOpenError(result.ok ? null : result.reason);
    });
  };

  // The provider only publishes a URL for the latest checkout request, so a superseded request
  // never reaches this effect. The visible button covers a blocked window.
  useEffect(() => {
    if (checkout.url) openCheckout(checkout.url);
  }, [checkout.url]);

  return (
    <section aria-labelledby={`${seatsId}-heading`} className="mt-6 rounded-xl border border-line-strong bg-canvas/70 p-4">
      <h2 id={`${seatsId}-heading`} className="text-sm font-semibold text-fg">Upgrade</h2>
      <div className="mt-3">
        <PriceTable />
      </div>
      <div className="mt-4 flex flex-wrap items-center gap-x-3 gap-y-2">
        <label htmlFor={seatsId} className="text-label leading-4 font-medium text-fg-secondary">Seats</label>
        <div className="flex items-center gap-1">
          <button
            type="button"
            aria-label="Fewer seats"
            disabled={seats <= MIN_SEATS}
            onClick={() => setSeats((value) => clampSeats(value - 1))}
            className="focus-ring h-8 w-8 rounded-lg border border-control text-fg disabled:opacity-40"
          >
            -
          </button>
          <input
            id={seatsId}
            type="number"
            inputMode="numeric"
            min={MIN_SEATS}
            max={MAX_SEATS}
            value={seats}
            onChange={(event) => setSeats(clampSeats(event.currentTarget.valueAsNumber))}
            className="focus-ring h-8 w-16 rounded-lg border border-control bg-surface px-2 text-center text-sm tabular-nums text-fg"
          />
          <button
            type="button"
            aria-label="More seats"
            disabled={seats >= MAX_SEATS}
            onClick={() => setSeats((value) => clampSeats(value + 1))}
            className="focus-ring h-8 w-8 rounded-lg border border-control text-fg disabled:opacity-40"
          >
            +
          </button>
        </div>
        <p className="text-label leading-4 text-fg-secondary" aria-live="polite">
          {seats} {seats === 1 ? 'seat' : 'seats'} at ${tier.usdPerSeatPerYear} = ${yearlyTotalUsd(seats)} per year, before tax
        </p>
      </div>
      <button
        type="button"
        disabled={checkout.busy}
        onClick={() => onStartCheckout(seats)}
        className="focus-ring mt-4 w-full rounded-lg bg-accent px-4 py-2.5 text-sm font-semibold text-muted hover:bg-accent disabled:cursor-wait disabled:opacity-60"
      >
        {checkout.busy ? 'Preparing checkout...' : 'Continue to checkout'}
      </button>
      {checkout.busy ? <p role="status" className="sr-only">Preparing checkout</p> : null}
      {checkout.error ? (
        <p role="alert" className="mt-3 rounded-lg border border-danger/30 bg-danger/10 px-3 py-2 text-label leading-4 text-danger">
          Checkout could not be started. {checkout.error}
        </p>
      ) : null}
      {checkout.url ? (
        <div role="status" className="mt-3 text-label leading-4 text-fg-secondary">
          <p>Checkout opened in your browser. If nothing opened, use the button.</p>
          <button
            type="button"
            onClick={() => openCheckout(checkout.url ?? '')}
            className="focus-ring mt-2 rounded-lg border border-accent px-3 py-1.5 font-semibold text-accent hover:bg-accent"
          >
            Open checkout in your browser
          </button>
          <p className="mt-2 break-all text-label text-muted">{checkout.url}</p>
          {openError ? <p role="alert" className="mt-2 text-danger">{openError}</p> : null}
        </div>
      ) : null}
      <p className="mt-3 text-label leading-4 text-muted">
        After you pay, return here. The license is checked again when this window regains focus.
      </p>
    </section>
  );
}
