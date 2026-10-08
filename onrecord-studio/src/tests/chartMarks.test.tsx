// @vitest-environment jsdom
import { afterEach, describe, expect, it } from 'vitest';
import type { ReactNode } from 'react';
import { cleanup, render } from '@testing-library/react';
import { HatchProvider } from '../components/charts/frame/VizDefs';
import { Bar } from '../components/charts/marks/Bar';
import { Dot } from '../components/charts/marks/Dot';
import { HeatCell } from '../components/charts/marks/HeatCell';
import { LineSeries } from '../components/charts/marks/Line';
import { absent, measured } from '../components/charts/core/nulls';
import { scaleLinear } from '../components/charts/core/scale';

afterEach(cleanup);

const x = scaleLinear([0, 100], [0, 200]);
const notRecorded = absent('not-recorded', 'the source has no value');

function plot(node: ReactNode) {
  return render(<svg><HatchProvider value="hatch">{node}</HatchProvider></svg>).container;
}

describe('marks', () => {
  it('draw an absent value as its dashed slot, never as a value mark', () => {
    const bar = plot(<Bar band={{ pos: 0, thickness: 12 }} value={notRecorded} scale={x} claim="counted" />);
    expect(bar.querySelector('path')).toBeNull();
    const slot = bar.querySelector('[data-absence="not-recorded"] rect') as SVGRectElement;
    expect(slot.style.strokeDasharray).toBe('3 2');
    cleanup();

    expect(plot(<Dot cx={10} cy={10} value={notRecorded} />).querySelector('circle')).toBeNull();
    cleanup();

    const cell = plot(<HeatCell x={0} y={0} width={20} height={20} value={notRecorded} />);
    expect((cell.querySelector('rect[data-absence="not-recorded"]') as SVGRectElement).style.fill).toBe('none');
  });

  it('draw a measured zero as a labelled zero-length mark', () => {
    const bar = plot(<Bar band={{ pos: 0, thickness: 12 }} value={measured(0)} scale={x} />);
    expect(bar.querySelector('line')).not.toBeNull();
    expect(bar.textContent).toBe('0');
  });

  it('break a line at a gap and still mark a lone observed point', () => {
    const y = scaleLinear([0, 10], [100, 0]);
    const points = [measured(2), notRecorded, measured(5), measured(6)].map((value, i) => ({ key: `p${i}`, x: i * 10, y: value }));
    const line = plot(<LineSeries points={points} x={x} y={y} />);
    const paths = [...line.querySelectorAll('path')].map((path) => path.getAttribute('d'));
    expect(paths).toEqual([`M${x(20)},${y(5)}L${x(30)},${y(6)}`]);
    // The lone first point has a marker (a surface halo plus the dot).
    expect([...line.querySelectorAll('circle')].some((circle) => circle.getAttribute('cx') === String(x(0)))).toBe(true);
  });
});
