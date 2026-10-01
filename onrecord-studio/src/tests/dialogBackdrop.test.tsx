// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, describe, expect, it } from 'vitest';
import DialogBackdrop from '../components/ui/DialogBackdrop';

function Host() {
  const [open, setOpen] = useState(false);
  return (
    <>
      <button type="button" onClick={() => setOpen(true)}>open</button>
      {open && (
        <DialogBackdrop labelledBy="t" onClose={() => setOpen(false)} className="x">
          <h2 id="t">Delete?</h2>
          <button type="button">first</button>
          <button type="button">last</button>
        </DialogBackdrop>
      )}
    </>
  );
}

describe('DialogBackdrop', () => {
  afterEach(cleanup);

  it('leaves focus on a child that claimed it with autoFocus', () => {
    render(
      <DialogBackdrop labelledBy="t" onClose={() => {}} className="x">
        <h2 id="t">Paste</h2>
        <textarea aria-label="body" autoFocus />
      </DialogBackdrop>,
    );
    expect(document.activeElement).toBe(screen.getByLabelText('body'));
  });

  it('keeps Tab inside, closes on Escape, and returns focus to the opener', () => {
    render(<Host />);
    const opener = screen.getByText('open');
    opener.focus();
    fireEvent.click(opener);
    const dialog = screen.getByRole('dialog');
    expect(document.activeElement).toBe(dialog);
    screen.getByText('last').focus();
    fireEvent.keyDown(dialog, { key: 'Tab' });
    expect(document.activeElement).toBe(screen.getByText('first'));
    fireEvent.keyDown(dialog, { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
    expect(document.activeElement).toBe(opener);
  });
});
