import { Fragment } from 'react';
import { paintFor, VIZ, type Claim, type MarkRole } from '../core/claim';
import { measureText } from '../core/layout';
import { ABSENCE_LABEL, isValue, segmentsOf, type Measured } from '../core/nulls';
import type { ContinuousScale } from '../core/scale';
import { useFrameMotion } from '../hooks/useChartMotion';
import { useSvgId } from '../frame/VizDefs';
import { LABEL_CLASS, MUTED_LABEL_CLASS } from './shared';

export interface LinePoint {
  key: string;
  /** The x value (a time in ms or a number) in the x scale's domain. */
  x: number;
  y: Measured;
  /** An incomplete measurement (a partial period): drawn as a hollow marker. */
  incomplete?: boolean;
}

type XScale = (v: number) => number;

interface Defined {
  key: string;
  px: number;
  py: number;
  incomplete?: boolean;
}

function runs(points: readonly LinePoint[], x: XScale, y: ContinuousScale): Defined[][] {
  return segmentsOf(points, (p) => isValue(p.y)).map((run) => run.map((p) => ({ key: p.key, px: x(p.x), py: y((p.y as { value: number }).value), incomplete: p.incomplete })));
}

function pathOf(run: readonly Defined[]): string {
  return run.map((p, i) => `${i === 0 ? 'M' : 'L'}${p.px},${p.py}`).join('');
}

export interface LineSeriesProps {
  points: readonly LinePoint[];
  x: XScale;
  y: ContinuousScale;
  claim?: Claim;
  role?: MarkRole;
  /** 'observed': a marker on every observed point; 'ends': first and last of each run only. */
  markers?: 'observed' | 'ends';
  /** Direct label at the series' last observed point. */
  label?: string;
  /** The label's y when placeLabels moved it off its point to avoid another series' label. */
  labelY?: number;
  opacity?: number;
  motionKey?: string;
}

/**
 * A 2px line with round joins that breaks at every absent point: each run of observed points is
 * its own path, so the line cannot interpolate across a gap. A run of one point is a marker. Each
 * gap gets a short muted tick on the axis and the words 'no data' where they fit.
 */
export function LineSeries({ points, x, y, claim = 'descriptive', role = 'primary', markers = 'ends', label, labelY, opacity, motionKey }: LineSeriesProps) {
  const { enterOnce } = useFrameMotion();
  const paint = paintFor(claim, 'line', role);
  const marker = paintFor(claim, role === 'comparison' ? 'hollow' : 'fill', role);
  const segments = runs(points, x, y);
  const enter = motionKey && !paint.dash ? enterOnce(motionKey) : false;
  const axisY = Math.max(...y.range);
  const gaps = segmentsOf(points, (p) => !isValue(p.y));
  const lastRun = segments[segments.length - 1];
  const last = lastRun?.[lastRun.length - 1];
  return (
    <g style={{ opacity }}>
      {segments.map((run) => (
        <Fragment key={run[0].key}>
          {run.length > 1 && (
            <path
              d={pathOf(run)}
              pathLength={enter ? 1 : undefined}
              className={enter ? 'viz-enter-draw' : undefined}
              style={{ fill: 'none', stroke: paint.stroke, strokeWidth: paint.strokeWidth, strokeDasharray: enter ? undefined : paint.dash, strokeLinejoin: 'round', strokeLinecap: 'round' }}
            />
          )}
          {run.map((p, i) => {
            const show = markers === 'observed' || run.length === 1 || i === 0 || i === run.length - 1 || p.incomplete;
            if (!show) return null;
            const hollow = p.incomplete || marker.fill === VIZ.surface || marker.fill === 'none';
            return (
              <g key={p.key}>
                <circle cx={p.px} cy={p.py} r={6} style={{ fill: VIZ.surface }} />
                <circle cx={p.px} cy={p.py} r={hollow ? 3.25 : 4} style={hollow ? { fill: VIZ.surface, stroke: paint.stroke, strokeWidth: 1.5 } : { fill: marker.fill }} />
              </g>
            );
          })}
        </Fragment>
      ))}
      {gaps.map((gap) => {
        const xs = gap.map((p) => x(p.x));
        const reasons = [...new Set(gap.map((p) => (isValue(p.y) ? '' : `${ABSENCE_LABEL[p.y.state]}: ${p.y.reason}`)))].join('; ');
        const mid = (xs[0] + xs[xs.length - 1]) / 2;
        const span = xs[xs.length - 1] - xs[0] + 24;
        return (
          <g key={`gap-${gap[0].key}`} data-gap="">
            <title>{reasons}</title>
            {xs.map((px) => <line key={px} x1={px} x2={px} y1={axisY - 6} y2={axisY} style={{ stroke: VIZ.context, strokeWidth: 1.5 }} />)}
            {span >= measureText('no data') && <text x={mid} y={axisY - 9} textAnchor="middle" className={MUTED_LABEL_CLASS}>no data</text>}
          </g>
        );
      })}
      {label && last && (
        <g>
          {labelY !== undefined && Math.abs(labelY - last.py) > 4 && (
            <line x1={last.px + 6} y1={last.py} x2={last.px + 12} y2={labelY} style={{ stroke: VIZ.baseline, strokeWidth: 1 }} />
          )}
          <text x={last.px + (labelY !== undefined && Math.abs(labelY - last.py) > 4 ? 14 : 8)} y={labelY ?? last.py} dy="0.32em" className={LABEL_CLASS}>{label}</text>
        </g>
      )}
    </g>
  );
}

/**
 * The area under a single count series: the role colour at 10% at the line fading to 0 at the
 * baseline, broken at gaps exactly like the line. Only where magnitude from zero matters; never on
 * a rate.
 */
export function AreaGradient({ points, x, y, role = 'primary' }: { points: readonly LinePoint[]; x: XScale; y: ContinuousScale; role?: MarkRole }) {
  const id = useSvgId('viz-area');
  const colour = paintFor('descriptive', 'line', role).stroke;
  const base = y(Math.max(Math.min(0, Math.max(...y.domain)), Math.min(...y.domain)));
  const top = Math.min(...y.range);
  return (
    <g aria-hidden="true">
      <defs>
        <linearGradient id={id} gradientUnits="userSpaceOnUse" x1={0} y1={top} x2={0} y2={base}>
          <stop offset="0%" style={{ stopColor: colour, stopOpacity: 0.1 }} />
          <stop offset="100%" style={{ stopColor: colour, stopOpacity: 0 }} />
        </linearGradient>
      </defs>
      {runs(points, x, y).filter((run) => run.length > 1).map((run) => (
        <path key={run[0].key} d={`${pathOf(run)}L${run[run.length - 1].px},${base}L${run[0].px},${base}Z`} style={{ fill: `url(#${id})` }} />
      ))}
    </g>
  );
}
