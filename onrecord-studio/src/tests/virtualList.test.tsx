// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import { cleanup, fireEvent, render, screen } from '@testing-library/react';
import VirtualList from '../components/ui/VirtualList';

afterEach(cleanup);

const items = Array.from({ length: 5000 }, (_, index) => `row ${index}`);

function mount() {
  return render(
    <VirtualList
      items={items}
      rowHeight={40}
      ariaLabel="rows"
      getKey={(item) => item}
      renderRow={(item, { tabIndex }) => <button type="button" tabIndex={tabIndex}>{item}</button>}
    />,
  );
}

describe('VirtualList', () => {
  it('mounts only the rows in the window while still describing the full set', () => {
    const { container } = mount();
    const rows = container.querySelectorAll('[data-row-index]');
    expect(rows.length).toBeGreaterThan(0);
    expect(rows.length).toBeLessThan(100);
    expect(rows[0].getAttribute('aria-setsize')).toBe('5000');
  });

  it('keeps one tab stop and moves focus by key, including to a row that was not mounted', () => {
    const { container } = mount();
    const first = screen.getByRole('button', { name: 'row 0' });
    expect(container.querySelectorAll('button[tabindex="0"]').length).toBe(1);
    first.focus();
    fireEvent.keyDown(first, { key: 'ArrowDown' });
    expect(document.activeElement?.textContent).toBe('row 1');

    fireEvent.keyDown(document.activeElement as Element, { key: 'End' });
    expect(document.activeElement?.textContent).toBe('row 4999');
    expect(container.querySelectorAll('[data-row-index]').length).toBeLessThan(100);
    expect(container.querySelectorAll('button[tabindex="0"]').length).toBe(1);
  });
});
