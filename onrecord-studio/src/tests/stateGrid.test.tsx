// @vitest-environment jsdom
import { cleanup, fireEvent, render, screen, within } from '@testing-library/react';
import { afterEach, describe, expect, it, vi } from 'vitest';
import type { StateGridCell } from '../components/charts/figures/StateGrid';
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
    const first = screen.getByRole('gridcell', { name: 'North a: 3' });
    expect(grid.querySelectorAll('[tabindex="0"]')).toHaveLength(1);
    first.focus();
    fireEvent.keyDown(first, { key: 'ArrowDown' });
    expect(document.activeElement).toBe(screen.getByRole('gridcell', { name: 'South a: 3' }));
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowRight' });
    expect(document.activeElement).toBe(screen.getByRole('gridcell', { name: 'South b: same' }));
    fireEvent.keyDown(document.activeElement!, { key: 'Enter' });
    expect(onActivate).not.toHaveBeenCalled();
    fireEvent.keyDown(document.activeElement!, { key: 'ArrowLeft' });
    fireEvent.keyDown(document.activeElement!, { key: 'Enter' });
    expect(onActivate).toHaveBeenCalledWith(1, 0);
  });

  it('draws absence as its empty cell with words, and prints its reason in the table, never a 0', () => {
    renderGrid();
    const gap = screen.getByRole('gridcell', { name: 'North c: not observed' });
    expect(gap.querySelector('[data-absence="not-recorded"]')).toBeTruthy();
    expect(gap.textContent).toContain('not observed');
    expect(screen.getByRole('gridcell', { name: 'North b: same' }).querySelector('[data-absence]')).toBeNull();
    fireEvent.click(screen.getByRole('button', { name: 'View as table' }));
    expect(screen.getAllByRole('cell', { name: 'No value recorded: the source did not load' })).toHaveLength(2);
  });

  it('keeps a changed count as visible text at a 600px half column, stepping down to its short form', () => {
    const width = vi.spyOn(HTMLElement.prototype, 'clientWidth', 'get').mockReturnValue(600);
    try {
    const dimensions = ['Documents', 'Findings', 'Figures', 'Redaction', 'Source', 'Source copies'];
    const changed = (n: number): StateGridCell => ({ kind: 'state', tone: 'lead', glyph: 'diamond', text: `${n} changed`, short: String(n), name: `changed ${n}`, actionable: true });
    const same: StateGridCell = { kind: 'state', tone: 'positive', glyph: 'check', text: 'same', name: 'same' };
    render(
      <StateGrid
        title="Watch outcomes"
        storageKey="test-watch"
        headline="One watch."
        sources={{ tables: ['watch_runs'] }}
        summary="One watch by six dimensions."
        status="ready"
        rowHeader="Watch"
        trailingHeader="Checked"
        columns={dimensions.map((label) => ({ key: label, label }))}
        rows={[{ key: 'w', label: 'Environmental Services Department', sublabel: 'Agency', trailing: { text: 'not checked yet' }, cells: [changed(128), changed(3), same, same, same, same] }]}
      />,
    );
    for (const n of [128, 3]) {
      const cell = screen.getByRole('gridcell', { name: `changed ${n}` });
      expect([...cell.querySelectorAll('text')].some((node) => node.textContent === `${n} changed` || node.textContent === String(n))).toBe(true);
    }
    } finally {
      width.mockRestore();
    }
  });
});
