// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { act, cleanup, fireEvent, render, renderHook, screen } from '@testing-library/react';
import { useRovingFocus } from '../components/charts/hooks/useRovingFocus';
import { useDrillDown } from '../components/charts/hooks/useDrillDown';
import { useChartMotion } from '../components/charts/hooks/useChartMotion';
import { useTooltip } from '../components/charts/hooks/useTooltip';
import { nearestIndex } from '../components/charts/hooks/useCrosshair';

afterEach(() => {
  cleanup();
  delete document.documentElement.dataset.motion;
});

function Grid({ onActivate, onEscape }: { onActivate(i: number, j?: number): void; onEscape(): void }) {
  const { getItemProps } = useRovingFocus({ shape: { rows: 3, cols: 4 }, onActivate, onEscape });
  return (
    <div role="grid">
      {[0, 1, 2].map((i) => (
        <div role="row" key={i}>
          {[0, 1, 2, 3].map((j) => <span role="gridcell" key={j} aria-label={`${i},${j}`} {...getItemProps(i, j)} />)}
        </div>
      ))}
    </div>
  );
}

describe('useRovingFocus', () => {
  it('is one tab stop moved by arrows, Home/End and Escape across a 2-D grid, never wrapping', () => {
    const onActivate = vi.fn();
    const onEscape = vi.fn();
    render(<Grid onActivate={onActivate} onEscape={onEscape} />);
    const cell = (i: number, j: number) => screen.getByLabelText(`${i},${j}`);
    const focused = () => document.activeElement?.getAttribute('aria-label');
    expect(screen.getAllByRole('gridcell').filter((c) => c.tabIndex === 0)).toHaveLength(1);
    act(() => cell(0, 0).focus());
    fireEvent.keyDown(cell(0, 0), { key: 'ArrowRight' });
    expect(focused()).toBe('0,1');
    fireEvent.keyDown(cell(0, 1), { key: 'ArrowDown' });
    expect(focused()).toBe('1,1');
    fireEvent.keyDown(cell(1, 1), { key: 'End' });
    expect(focused()).toBe('1,3');
    fireEvent.keyDown(cell(1, 3), { key: 'ArrowRight' });
    expect(focused()).toBe('1,3');
    fireEvent.keyDown(cell(1, 3), { key: 'Home' });
    expect(focused()).toBe('1,0');
    fireEvent.keyDown(cell(1, 0), { key: 'End', ctrlKey: true });
    expect(focused()).toBe('2,3');
    fireEvent.keyDown(cell(2, 3), { key: 'PageUp' });
    expect(focused()).toBe('0,3');
    expect(screen.getAllByRole('gridcell').filter((c) => c.tabIndex === 0).map((c) => c.getAttribute('aria-label'))).toEqual(['0,3']);
    fireEvent.keyDown(cell(0, 3), { key: 'Enter' });
    expect(onActivate).toHaveBeenCalledWith(0, 3);
    fireEvent.keyDown(cell(0, 3), { key: 'Escape' });
    expect(onEscape).toHaveBeenCalledTimes(1);
  });
});

describe('useDrillDown', () => {
  it('expands synchronously where WebKit has no view transitions (macOS 13 and 14)', () => {
    expect((document as Document & { startViewTransition?: unknown }).startViewTransition).toBeUndefined();
    document.documentElement.dataset.motion = 'full';
    const { result } = renderHook(() => useDrillDown());
    act(() => result.current.toggle('agency-102'));
    expect(result.current.expanded).toBe('agency-102');
    expect(result.current.buttonProps('agency-102')['aria-expanded']).toBe(true);
    expect(result.current.regionProps('agency-102')['data-open']).toBe('');
    expect(result.current.buttonProps('agency-102')['aria-controls']).toBe(result.current.regionProps('agency-102').id);
  });
});

describe('useChartMotion', () => {
  it('animates nothing under data-motion=reduce, and a mark only on its first commit otherwise', () => {
    document.documentElement.dataset.motion = 'reduce';
    const reduced = renderHook(() => useChartMotion());
    expect(reduced.result.current.reduce).toBe(true);
    expect(reduced.result.current.enterOnce('bar-a')).toBe(false);

    document.documentElement.dataset.motion = 'full';
    const full = renderHook(() => useChartMotion());
    expect(full.result.current.enterOnce('bar-a')).toBe(true);
    full.rerender();
    expect(full.result.current.enterOnce('bar-a')).toBe(false);
  });
});

describe('useTooltip', () => {
  it('renders content from data as text, never as markup', () => {
    const hostile = '<img src=x onerror="window.__pwned=1">Harborview';
    function Probe() {
      const tooltip = useTooltip();
      return (
        <>
          <button type="button" onClick={(event) => tooltip.show(hostile, event.currentTarget.getBoundingClientRect())}>mark</button>
          {tooltip.node}
        </>
      );
    }
    render(<Probe />);
    fireEvent.click(screen.getByRole('button', { name: 'mark' }));
    const tip = screen.getByRole('tooltip', { hidden: true });
    expect(tip.textContent).toBe(hostile);
    expect(tip.querySelector('img')).toBeNull();
  });
});

describe('useCrosshair', () => {
  it('snaps to the nearest observed x', () => {
    const xs = [0, 10, 30, 70];
    expect([-5, 4, 6, 19, 21, 50, 51, 200].map((x) => nearestIndex(xs, x))).toEqual([0, 0, 1, 1, 2, 2, 3, 3]);
  });
});
