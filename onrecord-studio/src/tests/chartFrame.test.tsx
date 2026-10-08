// @vitest-environment jsdom
import { afterEach, describe, expect, it, vi } from 'vitest';
import { cleanup, render, screen, within } from '@testing-library/react';

// Node 26's flag-gated `localStorage` shadows jsdom's; ChartFrame remembers the table toggle there.
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

import ChartFrame, { type ChartFrameProps } from '../components/charts/frame/ChartFrame';
import { ReferenceLine } from '../components/charts/frame/ReferenceLine';
import { absent } from '../components/charts/core/nulls';
import type { YardstickLabel } from '../components/charts/core/claim';
import { Axis } from '../components/charts/frame/Axis';
import { scaleLinear } from '../components/charts/core/scale';
import { LABEL_FONT, measureText } from '../components/charts/core/layout';

afterEach(() => {
  cleanup();
  localStorage.clear();
});

const rows = [
  { department: 'Environmental Services', closure: 41.2 },
  { department: 'Police', closure: absent('not-recorded', 'no request closed with a recorded date') },
  { department: 'Calls without a disposition', closure: absent('excluded', 'blank or cancelled dispositions') },
];

function frame(overrides: Partial<ChartFrameProps> = {}) {
  return (
    <ChartFrame
      title="Average closure time"
      headline="Environmental Services averaged 41.2 days."
      sources={{ tables: ['agency_performance_profiles'] }}
      summary="One department measured, one with no recorded closures, one outside this measure."
      status="ready"
      table={{ caption: 'Average closure time by department', columns: [{ key: 'department', label: 'Department' }, { key: 'closure', label: 'Average closure', kind: 'days' }], rows }}
      tableDefault="table"
      minHeight={120}
      {...overrides}
    >
      {() => <rect data-testid="plotted" width={10} height={10} />}
    </ChartFrame>
  );
}

describe('ChartFrame', () => {
  it('twins exactly the plotted rows plus each absent and excluded row with its reason', () => {
    render(frame());
    const table = screen.getByRole('table', { name: 'Average closure time by department' });
    const bodyRows = within(table).getAllByRole('row').slice(1).map((row) => within(row).getAllByRole('cell').map((cell) => cell.textContent));
    expect(bodyRows).toEqual([
      ['Environmental Services', '41.2 d'],
      ['Police', 'No value recorded: no request closed with a recorded date'],
      ['Calls without a disposition', 'Outside this measure: blank or cancelled dispositions'],
    ]);
  });

  it('appends one legend entry per absence state present in the data', () => {
    render(frame({ tableDefault: 'chart', legend: [{ key: 'agency', label: 'This agency', swatch: 'dot' }] }));
    const legend = screen.getByRole('list', { name: 'Legend' });
    expect(within(legend).getAllByRole('listitem').map((item) => item.textContent)).toEqual(['This agency', 'No value recorded', 'Outside this measure']);
  });

  it('keeps the previous render at half opacity while stale, with no skeleton', () => {
    const { container } = render(frame({ status: 'stale', tableDefault: 'chart' }));
    const plotted = screen.getByTestId('plotted');
    expect((plotted.closest('svg')?.parentElement as HTMLElement).style.opacity).toBe('0.5');
    expect(container.querySelector('.skeleton')).toBeNull();
    expect(screen.getByRole('status').textContent).toMatch(/Controls changed/);
  });

  it('draws no data while loading: text only', () => {
    const { container } = render(frame({ status: 'loading', headline: null, tableDefault: 'chart' }));
    expect(container.querySelector('svg rect, svg path, svg circle')).toBeNull();
    expect(screen.queryByRole('table')).toBeNull();
  });
});

describe('ReferenceLine', () => {
  it('refuses a yardstick whose label does not say it is a reference', () => {
    const silence = vi.spyOn(console, 'error').mockImplementation(() => {});
    const label = '72 h before start' as YardstickLabel;
    expect(() => render(<svg><ReferenceLine kind="yardstick" label={label} at={10} extent={[0, 100]} axis="x" /></svg>)).toThrow(/reference/);
    silence.mockRestore();
    render(<svg><ReferenceLine kind="yardstick" label="72 h before start (reference; no notice rule is evaluated)" at={10} extent={[0, 100]} axis="x" /></svg>);
    expect(screen.getByText(/72 h before start \(reference/)).toBeTruthy();
  });
});

describe('Axis', () => {
  // A label's drawn extent after edge anchoring, by the same measure the axis uses.
  function extents(container: HTMLElement): Array<{ label: string; left: number; right: number }> {
    return [...container.querySelectorAll('text')].map((node) => {
      const label = node.textContent ?? '';
      const width = measureText(label, LABEL_FONT);
      const x = Number(node.getAttribute('x'));
      const anchor = node.getAttribute('text-anchor');
      const left = anchor === 'start' ? x : anchor === 'end' ? x - width : x - width / 2;
      return { label, left, right: left + width };
    });
  }

  it('never draws two bottom labels over each other, including an edge label anchored inward', () => {
    for (let width = 60; width <= 420; width += 7) {
      const { container, unmount } = render(
        <svg>
          <Axis orient="bottom" offset={0} scale={scaleLinear([0, 1000], [0, width])} kind="days" tickCount={6} />
        </svg>,
      );
      const drawn = extents(container);
      for (let i = 1; i < drawn.length; i++) expect(drawn[i].left, `width ${width}: ${drawn[i - 1].label} / ${drawn[i].label}`).toBeGreaterThanOrEqual(drawn[i - 1].right);
      // The domain's end stays labelled once the two edge labels can both fit.
      if (width >= 100) expect(drawn[drawn.length - 1].label).toBe('1,000 d');
      unmount();
    }
  });
});
