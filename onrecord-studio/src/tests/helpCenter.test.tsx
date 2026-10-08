// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, screen, waitFor, within } from '@testing-library/react';

// Node 26's flag-gated `localStorage` shadows jsdom's, so replace it before store import.
vi.hoisted(() => {
  const entries = new Map<string, string>();
  Object.defineProperty(globalThis, 'localStorage', {
    configurable: true,
    value: {
      getItem: (key: string) => entries.get(key) ?? null,
      setItem: (key: string, value: string) => void entries.set(key, String(value)),
      removeItem: (key: string) => void entries.delete(key),
      clear: () => entries.clear(),
      key: (index: number) => [...entries.keys()][index] ?? null,
      get length() {
        return entries.size;
      },
    },
  });
});

vi.mock('@tauri-apps/api/core', () => ({ isTauri: () => false, invoke: () => Promise.resolve(null) }));

import CommandPalette from '../components/dashboard/CommandPalette';
import HelpButton from '../components/help/HelpButton';
import HelpCenter, { HelpDialog } from '../components/help/HelpCenter';
import { openHelp } from '../components/help/helpEvents';
import { useAppStore } from '../store/useAppStore';

Element.prototype.scrollIntoView = vi.fn();

afterEach(() => {
  cleanup();
  useAppStore.setState({ workspaceView: 'overview' });
});

describe('HelpDialog', () => {
  it('is a labelled modal dialog with a labelled search field and topic list', () => {
    render(<HelpDialog initialTopicId={null} onClose={() => {}} />);
    const dialog = screen.getByRole('dialog', { name: 'OnRecord Studio Help' });
    expect(dialog.getAttribute('aria-modal')).toBe('true');
    expect(screen.getByLabelText('Search help topics')).toBeTruthy();
    expect(screen.getByRole('navigation', { name: 'Help topics' })).toBeTruthy();
    expect(screen.getByRole('button', { name: 'Close' })).toBeTruthy();
    expect(screen.getByRole('status').textContent).toBe('');
  });

  it('opens on the requested topic and shows its content', () => {
    render(<HelpDialog initialTopicId="statutory-deadlines" onClose={() => {}} />);
    expect(screen.getByRole('heading', { level: 2, name: 'Statutory deadlines' })).toBeTruthy();
    const current = within(screen.getByRole('navigation', { name: 'Help topics' })).getByRole('button', { name: 'Statutory deadlines' });
    expect(current.getAttribute('aria-current')).toBe('true');
  });

  it('filters by the search text, selects the best match, and reports the count', () => {
    render(<HelpDialog initialTopicId={null} onClose={() => {}} />);
    fireEvent.change(screen.getByLabelText('Search help topics'), { target: { value: 'openrouter' } });
    const list = screen.getByRole('navigation', { name: 'Help topics' });
    expect(within(list).getAllByRole('button').length).toBeGreaterThan(0);
    expect(within(list).queryByRole('button', { name: /Keyboard shortcuts/ })).toBeNull();
    expect(screen.getByRole('status').textContent).toMatch(/topics? match/);
    expect(screen.getByRole('article').textContent).toContain('OpenRouter');
  });

  it('says so when nothing matches, and treats punctuation as text', () => {
    render(<HelpDialog initialTopicId={null} onClose={() => {}} />);
    const input = screen.getByLabelText('Search help topics');
    fireEvent.change(input, { target: { value: 'zzzzqqqq' } });
    expect(screen.getByText(/No topic matches/)).toBeTruthy();
    expect(() => fireEvent.change(input, { target: { value: '(*[' } })).not.toThrow();
  });

  it('navigates to a related topic', () => {
    render(<HelpDialog initialTopicId="start-here" onClose={() => {}} />);
    const related = within(screen.getByRole('region', { name: 'Related topics' }));
    fireEvent.click(related.getByRole('button', { name: 'Keyboard shortcuts' }));
    expect(screen.getByRole('heading', { level: 2, name: 'Keyboard shortcuts' })).toBeTruthy();
  });

  it('closes on Escape and on the close button', () => {
    const onClose = vi.fn();
    render(<HelpDialog initialTopicId={null} onClose={onClose} />);
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    fireEvent.click(screen.getByRole('button', { name: 'Close' }));
    expect(onClose).toHaveBeenCalledTimes(2);
  });

  it('describes the evidence ladder figure with a text alternative', () => {
    render(<HelpDialog initialTopicId="evidence-ladder" onClose={() => {}} />);
    const figure = screen.getByRole('img', { name: /^Evidence ladder/ });
    expect(figure.textContent).toContain('Legal claim: counted violation');
    expect(figure.textContent).toContain('Unassessed');
  });
});

describe('opening Help', () => {
  it('opens from the event and closes again', () => {
    render(<HelpCenter />);
    expect(screen.queryByRole('dialog')).toBeNull();
    act(() => openHelp('data-handling'));
    expect(screen.getByRole('heading', { level: 2, name: /Data handling/ })).toBeTruthy();
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    expect(screen.queryByRole('dialog')).toBeNull();
  });

  it('opens the topic for whichever view is showing from the header control', () => {
    useAppStore.setState({ workspaceView: 'evidence' });
    render(<><HelpButton /><HelpCenter /></>);
    fireEvent.click(screen.getByRole('button', { name: 'Open help for Evidence view' }));
    expect(screen.getByRole('heading', { level: 2, name: 'Evidence view' })).toBeTruthy();
  });

  it('opens from Cmd/Ctrl+Shift+/ and from the command palette', async () => {
    render(<><CommandPalette /><HelpCenter /></>);
    fireEvent.keyDown(window, { key: '?', ctrlKey: true, metaKey: true, shiftKey: true });
    expect(screen.getByRole('dialog', { name: 'OnRecord Studio Help' })).toBeTruthy();
    fireEvent.keyDown(screen.getByRole('dialog'), { key: 'Escape' });
    expect(screen.queryByRole('dialog', { name: 'OnRecord Studio Help' })).toBeNull();

    fireEvent.keyDown(window, { key: 'k', ctrlKey: true, metaKey: true });
    fireEvent.change(screen.getByRole('combobox'), { target: { value: 'open help' } });
    fireEvent.keyDown(window, { key: 'Enter' });
    await waitFor(() => expect(screen.getByRole('dialog', { name: 'OnRecord Studio Help' })).toBeTruthy());
  });
});
