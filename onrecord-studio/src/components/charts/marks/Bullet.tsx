import { paintFor, VIZ, type Claim } from '../core/claim';
import { measureText } from '../core/layout';
import { isValue, type Measured } from '../core/nulls';
import type { ContinuousScale } from '../core/scale';
import { AbsentSlot, LABEL_CLASS, MUTED_LABEL_CLASS, MarkGroup, hBarPath, type MarkInteraction } from './shared';
import type { BandPlacement } from './Bar';

export interface BulletTick {
  key: string;
  at: Measured;
  label: string;
}

export interface BulletProps {
  band: BandPlacement;
  scale: ContinuousScale;
  /** The full track: e.g. retrieved contract authority. */
  track: Measured;
  /** An inner band of the track: e.g. base authority before amendments. */
  base?: Measured;
  /** The measured bar: e.g. posted payments. */
  bar: Measured;
  ticks?: readonly BulletTick[];
  claim?: Claim;
  /** Printed at the bar's end ('$1.24M paid'). */
  barLabel?: string;
  /**
   * Printed in ink when the bar runs past the track ('+$236,200 over retrieved authority'). An
   * overrun is a comparison of retrieved figures, shown by position and words, never by crimson.
   */
  overrunLabel?: string;
  /** Right edge the bar's words may reach, px. Default: the end of the scale's range. */
  labelLimit?: number;
  interaction?: MarkInteraction;
}

/**
 * A bullet row: a receding track, an optional inner base band, a thinner ink bar for the measure,
 * and target ticks. An absent track is its slot ('not retrieved'), and the bar is still drawn
 * against the axis.
 */
export function Bullet({ band, scale, track, base, bar, ticks = [], claim = 'descriptive', barLabel, overrunLabel, labelLimit, interaction }: BulletProps) {
  const zero = scale(Math.max(Math.min(0, Math.max(...scale.domain)), Math.min(...scale.domain)));
  const { pos, thickness } = band;
  const barThickness = Math.max(4, Math.round(thickness * 0.42));
  const barTop = pos + (thickness - barThickness) / 2;
  const paint = paintFor(claim, claim === 'statistical' ? 'outline' : 'fill', 'primary');
  const trackEnd = isValue(track) ? scale(track.value) : null;
  const barEnd = isValue(bar) ? scale(bar.value) : null;
  const over = trackEnd !== null && barEnd !== null && barEnd > trackEnd;
  const text = over && overrunLabel ? overrunLabel : barLabel;
  const textX = Math.max(barEnd ?? zero, trackEnd ?? zero) + 6;
  const fitsRight = text ? textX + measureText(text) <= (labelLimit ?? Math.max(...scale.range)) + 2 : false;
  return (
    <MarkGroup interaction={interaction} box={{ x: zero, y: pos, width: Math.max(trackEnd ?? 0, barEnd ?? 0, zero + 1) - zero, height: thickness }}>
      {isValue(track)
        ? <rect x={zero} y={pos} width={Math.max(0, (trackEnd ?? zero) - zero)} height={thickness} rx={2} style={{ fill: VIZ.track }} />
        : <AbsentSlot absence={track} box={{ x: zero, y: pos, width: Math.max(...scale.range) - zero, height: thickness }} text="not retrieved" align="end" />}
      {base && isValue(base) && (
        <rect x={zero} y={pos} width={Math.max(0, scale(base.value) - zero)} height={thickness} rx={2} style={{ fill: 'var(--viz-ord-1)', opacity: 0.45 }} />
      )}
      {isValue(bar) && barEnd !== null && (
        barEnd === zero
          ? <line x1={zero} x2={zero} y1={barTop} y2={barTop + barThickness} style={{ stroke: VIZ.ink, strokeWidth: 1.5 }} />
          : <path d={hBarPath(zero, barEnd, barTop, barThickness, 2)} style={{ fill: paint.fill, stroke: paint.stroke, strokeWidth: paint.strokeWidth }} />
      )}
      {ticks.map((tick) => {
        if (!isValue(tick.at)) return null;
        const at = Math.round(scale(tick.at.value)) + 0.5;
        return (
          <line key={tick.key} x1={at} x2={at} y1={pos - 2} y2={pos + thickness + 2} style={{ stroke: VIZ.ink, strokeWidth: 2 }}>
            <title>{tick.label}</title>
          </line>
        );
      })}
      {!isValue(bar) && <text x={zero + 4} y={pos + thickness / 2} dy="0.32em" className={MUTED_LABEL_CLASS}>{bar.state === 'not-covered' ? 'not checked' : 'not recorded'}</text>}
      {text && fitsRight && (
        <text x={textX} y={pos + thickness / 2} dy="0.32em" className={over ? 'viz-num fill-fg text-[11px] font-medium' : LABEL_CLASS}>{text}</text>
      )}
    </MarkGroup>
  );
}
