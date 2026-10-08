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
import Button from '../ui/Button';
import { Input } from '../ui/Field';
import Table, { Td, Th } from '../ui/Table';
import type { CheckoutState } from './LicenseProvider';

/** Price facts, rendered only from `config/pricing.ts`. */
export function PriceTable() {
  return (
    <div>
      <Table label="Price per seat per year">
        <thead>
          <tr>
            <Th>Seats</Th>
            <Th numeric>Per seat, per year</Th>
          </tr>
        </thead>
        <tbody>
          {PRICE_TIERS.map((tier) => (
            <tr key={tier.minSeats}>
              <Td className="tabular-nums">{tierLabel(tier)}</Td>
              <Td numeric>${tier.usdPerSeatPerYear}</Td>
            </tr>
          ))}
        </tbody>
      </Table>
      <p className="mt-2 text-caption text-muted">{PRICE_TERMS}</p>
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
    <section aria-labelledby={`${seatsId}-heading`} className="mt-6">
      <h2 id={`${seatsId}-heading`} className="text-heading text-fg">Upgrade</h2>
      <div className="mt-3">
        <PriceTable />
      </div>
      <div className="mt-4 flex flex-wrap items-center gap-x-3 gap-y-2">
        <label htmlFor={seatsId} className="text-control text-fg-secondary">Seats</label>
        <div className="flex items-center gap-1">
          <Button
            aria-label="Fewer seats"
            disabled={seats <= MIN_SEATS}
            onClick={() => setSeats((value) => clampSeats(value - 1))}
            className="w-8 px-0"
          >
            <span aria-hidden="true">−</span>
          </Button>
          <Input
            id={seatsId}
            type="number"
            inputMode="numeric"
            min={MIN_SEATS}
            max={MAX_SEATS}
            value={seats}
            onChange={(event) => setSeats(clampSeats(event.currentTarget.valueAsNumber))}
            className="w-16 text-center tabular-nums"
          />
          <Button
            aria-label="More seats"
            disabled={seats >= MAX_SEATS}
            onClick={() => setSeats((value) => clampSeats(value + 1))}
            className="w-8 px-0"
          >
            <span aria-hidden="true">+</span>
          </Button>
        </div>
        <p className="text-caption tabular-nums text-fg-secondary" aria-live="polite">
          {seats} {seats === 1 ? 'seat' : 'seats'} at ${tier.usdPerSeatPerYear} = ${yearlyTotalUsd(seats)} per year, before tax
        </p>
      </div>
      <Button
        variant="primary"
        size="lg"
        disabled={checkout.busy}
        onClick={() => onStartCheckout(seats)}
        className="mt-4 w-full"
      >
        {checkout.busy ? 'Preparing checkout…' : 'Continue to checkout'}
      </Button>
      {checkout.busy ? <p role="status" className="sr-only">Preparing checkout</p> : null}
      {checkout.error ? (
        <p role="alert" className="mt-3 rounded-lg border border-danger/30 bg-danger/10 px-3 py-2 text-body text-danger">
          Checkout could not be started. {checkout.error}
        </p>
      ) : null}
      {checkout.url ? (
        <div role="status" className="mt-3 text-body text-fg-secondary">
          <p>Checkout opened in your browser. If nothing opened, use the button.</p>
          <Button size="sm" onClick={() => openCheckout(checkout.url ?? '')} className="mt-2">
            Open checkout in your browser
          </Button>
          <p className="mt-2 break-all text-caption text-muted">{checkout.url}</p>
          {openError ? <p role="alert" className="mt-2 text-danger">{openError}</p> : null}
        </div>
      ) : null}
      <p className="mt-3 text-caption text-muted">
        After you pay, return here. The license is checked again when this window regains focus.
      </p>
    </section>
  );
}
