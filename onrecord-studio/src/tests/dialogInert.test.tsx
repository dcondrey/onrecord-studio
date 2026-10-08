// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import Dialog, { DialogHeader } from '../components/ui/Dialog';

function Page() {
  const [open, setOpen] = useState(false);
  const [nested, setNested] = useState(false);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>Open</button>
      {open && (
        <Dialog onClose={() => setOpen(false)}>
          <DialogHeader title="Outer" />
          <button type="button" onClick={() => setNested(true)}>Nested</button>
          {nested && (
            <Dialog onClose={() => setNested(false)}>
              <DialogHeader title="Inner" />
            </Dialog>
          )}
        </Dialog>
      )}
    </>
  );
}

afterEach(cleanup);

describe('modal dialogs', () => {
  it('make the page behind them inert until the last one closes, then return focus to the opener', () => {
    const { container } = render(<Page />);
    const opener = screen.getByRole('button', { name: 'Open' });
    opener.focus();
    fireEvent.click(opener);
    expect(container.hasAttribute('inert')).toBe(true);
    expect(container.getAttribute('aria-hidden')).toBe('true');

    fireEvent.click(screen.getByRole('button', { name: 'Nested' }));
    const outer = screen.getAllByRole('dialog', { hidden: true })[0].closest('body > *');
    expect(outer?.hasAttribute('inert')).toBe(true);

    fireEvent.click(screen.getAllByRole('button', { name: 'Close' }).at(-1)!);
    expect(outer?.hasAttribute('inert')).toBe(false);
    expect(container.hasAttribute('inert')).toBe(true);

    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(container.hasAttribute('inert')).toBe(false);
    expect(container.hasAttribute('aria-hidden')).toBe(false);
    expect(document.activeElement).toBe(opener);
  });
});
