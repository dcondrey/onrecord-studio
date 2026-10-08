// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import { absent } from '../components/charts/core/nulls';
import StateGrid, { type StateGridRow } from '../components/charts/figures/StateGrid';

afterEach(cleanup);

const rows: StateGridRow[] = ['North', 'South'].map((label) => ({
  key: label,
  label,
  cells: [
    { kind: 'count', value: 3, step: 4, glyph: 'circle', text: '3', name: `${label} a: 3`, actionable: true },
    { kind: 'state', tone: 'positive', glyph: 'check', text: 'same', name: `${label} b: same` },
    { kind: 'gap', absence: absent('not-recorded', 'the source did not load'), glyph: 'ring', text: 'not observed', name: `${label} c: not observed` },
  ],
}));

function renderGrid(onActivate = vi.fn()) {
  render(
    <StateGrid
      title="Test grid"
      storageKey="test-grid"
      headline="Two rows."
      sources={{ tables: ['t'] }}
      summary="Two rows by three columns."
      status="ready"
      rowHeader="Row"
      columns={[{ key: 'a', label: 'A' }, { key: 'b', label: 'B' }, { key: 'c', label: 'C' }]}
      rows={rows}
      onActivate={onActivate}
    />,
  );
  return onActivate;
}

describe('StateGrid', () => {
  it('is one tab stop moved through in two dimensions, activating only actionable cells', () => {
    const onActivate = renderGrid();
    const grid = screen.getByRole('grid', { name: 'Test grid' });
    expect(within(grid).getAllByRole('row')).toHaveLength(3);
    const first = screen.getByRole('img', { name: 'North a: 3' });
    expect(grid.querySelectorAll('[tabindex="0"]')).toHaveLength(1);
    first.focus();
    fireEvent.keyDown(first, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(screen.getByRole('img', { name: 'South a: 3' }));
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowRight' });
    expect(document.activeElement).toBe(screen.getByRole('img', { name: 'South b: same' }));
    fireEvent.keyDown(document.activeElement!, { key: 'Enter' });
    expect(onActivate).not.toHaveBeenCalled();
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowLeft' });
    fireEvent.keyDown(document.activeElement!, { key: 'Enter' });
    expect(onActivate).toHaveBeenCalledWith(1, 0);
  });

  it('draws absence as its empty cell with words, and prints its reason in the table, never a 0', () => {
    renderGrid();
    const gap = screen.getByRole('img', { name: 'North c: not observed' });
    expect(gap.querySelector('[data-absence="not-recorded"]')).toBeTruthy();
    expect(gap.textContent).toContain('not observed');
    expect(screen.getByRole('img', { name: 'North b: same' }).querySelector('[data-absence]')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'View as table' }));
    expect(screen.getAllByRole('cell', { name: 'No value recorded: the source did not load' })).toHaveLength(2);
  });
});
