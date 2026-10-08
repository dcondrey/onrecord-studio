// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import { useState } from 'react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { ClaimBasisBadge } from '../components/ui/FamilyBadge';
import { PLAIN_FINDING_TEXT } from '../content/plainLanguage';
import { isLegalClaimKind, type FindingKind } from '../types/findings';
import { withViewTransition } from '../utils/viewTransition';
import { Select } from '../components/ui/Field';
import SegmentedControl from '../components/ui/SegmentedControl';
import Stat from '../components/ui/Stat';
import Time from '../components/ui/Time';

afterEach(cleanup);

describe('Stat', () => {
  it('states an unmeasured value in words and never as a digit', () => {
    const { container } = render(<Stat label="Documents saved" value={null} unavailableReason="No source has synced." />);
    expect(container.textContent).toContain('Not measured');
    expect(container.textContent).not.toMatch(/\d/);
  });
});

describe('Select', () => {
  it('is named by its visible label', () => {
    render(<Select label="Portal"><option>All</option></Select>);
    expect(screen.getByRole('combobox', { name: 'Portal' })).toBeTruthy();
  });
});

describe('SegmentedControl', () => {
  function Host() {
    const [value, setValue] = useState<'a' | 'b' | 'c'>('a');
    return (
      <SegmentedControl
        label="View"
        mode="tabs"
        value={value}
        onChange={setValue}
        options={[{ value: 'a', label: 'A' }, { value: 'b', label: 'B' }, { value: 'c', label: 'C' }]}
      />
    );
  }

  it('moves selection with the arrow, Home and End keys and keeps a single tab stop', () => {
    render(<Host />);
    const list = screen.getByRole('tablist', { name: 'View' });
    const selected = () => screen.getAllByRole('tab').filter((tab) => tab.getAttribute('aria-selected') === 'true').map((tab) => tab.textContent);
    const stops = () => screen.getAllByRole('tab').filter((tab) => tab.tabIndex === 0).length;
    fireEvent.keyDown(list, { key: 'ArrowRight' });
    expect(selected()).toEqual(['B']);
    expect(document.activeElement?.textContent).toBe('B');
    fireEvent.keyDown(list, { key: 'End' });
    expect(selected()).toEqual(['C']);
    fireEvent.keyDown(list, { key: 'ArrowRight' });
    expect(selected()).toEqual(['A']);
    fireEvent.keyDown(list, { key: 'Home' });
    expect(selected()).toEqual(['A']);
    expect(stops()).toBe(1);
  });
});

describe('Time', () => {
  it('shows a recorded calendar date as that date west of UTC', () => {
    const zone = process.env.TZ;
    process.env.TZ = 'America/Los_Angeles';
    try {
      const { container } = render(<Time value="2026-10-09" />);
      expect(container.textContent).toMatch(/\b9\b/);
      expect(container.textContent).not.toMatch(/\b8\b/);
    } finally {
      process.env.TZ = zone;
    }
  });
});

describe('ClaimBasisBadge', () => {
  it('renders the solid counted-violation badge for statutory kinds and for nothing else', () => {
    for (const kind of Object.keys(PLAIN_FINDING_TEXT) as FindingKind[]) {
      const { container, unmount } = render(<ClaimBasisBadge kind={kind} />);
      const solid = container.querySelector('.bg-danger') !== null;
      expect({ kind, solid }).toEqual({ kind, solid: isLegalClaimKind(kind) });
      expect(container.textContent?.includes('Counted violation')).toBe(isLegalClaimKind(kind));
      unmount();
    }
  });
});

describe('withViewTransition', () => {
  it('skips the transition under reduced motion and applies the update synchronously', () => {
    const start = vi.fn((update: () => void) => update());
    Object.assign(document, { startViewTransition: start });
    document.documentElement.dataset.motion = 'reduce';
    try {
      let applied = false;
      withViewTransition(() => { applied = true; });
      expect(applied).toBe(true);
      expect(start).not.toHaveBeenCalled();
    } finally {
      delete (document as { startViewTransition?: unknown }).startViewTransition;
      delete document.documentElement.dataset.motion;
    }
  });
});
