// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import Dialog, { DialogHeader } from '../components/ui/Dialog';
import DialogBackdrop from '../components/ui/DialogBackdrop';

// A backdrop-filter (the app header has one) makes its element the containing block for fixed
// descendants, so a dialog left in place would be laid out inside that element's box.
function Host({ backdrop = false }: { backdrop?: boolean }) {
  const [open, setOpen] = useState(false);
  return (
    <header style={{ backdropFilter: 'blur(1px)' }}>
      <button type="button" onClick={() => setOpen(true)}>open</button>
      {open && !backdrop && (
        <Dialog onClose={() => setOpen(false)}>
          <DialogHeader title="Local profile" />
        </Dialog>
      )}
      {open && backdrop && (
        <DialogBackdrop label="Purge" onClose={() => setOpen(false)} className="fixed inset-0">
          <button type="button">confirm</button>
        </DialogBackdrop>
      )}
    </header>
  );
}

describe('Dialog', () => {
  afterEach(cleanup);

  it('renders outside a filtered ancestor, is named by its title, and closes back to the opener', () => {
    render(<Host />);
    const opener = screen.getByText('open');
    opener.focus();
    fireEvent.click(opener);
    const dialog = screen.getByRole('dialog', { name: 'Local profile' });
    expect(dialog.parentElement?.parentElement).toBe(document.body);
    expect(opener.closest('header')?.contains(dialog)).toBe(false);
    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(document.activeElement).toBe(opener);
  });

  it('portals the legacy backdrop the same way', () => {
    render(<Host backdrop />);
    fireEvent.click(screen.getByText('open'));
    expect(screen.getByRole('dialog', { name: 'Purge' }).parentElement).toBe(document.body);
  });
});
