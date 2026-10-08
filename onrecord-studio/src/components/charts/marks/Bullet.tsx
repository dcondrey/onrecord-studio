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
  /** A lighter segment continuing the bar from its end: e.g. encumbered beside actual. */
  extension?: Measured;
  ticks?: readonly BulletTick[];
  claim?: Claim;
  /** Printed at the bar's end ('$1.24M paid'). */
  barLabel?: string;
  /**
   * Printed in ink when the bar runs past the track ('+$236,200 over retrieved authority'). An
   * overrun is a comparison of retrieved figures, shown by position and words, never by crimson.
   */
  overrunLabel?: string;
  /** Words for an absent track, after the bar's words. Default from its absence; '' when the bar label already says it. */
  trackSlotText?: string;
  /** Words for an absent bar. Default from its absence; '' when the bar label already says it. */
  barSlotText?: string;
  /** Right edge the bar's words may reach, px. Default: the end of the scale's range. */
  labelLimit?: number;
  interaction?: MarkInteraction;
}

const STUB = 22;
const ABSENT_WORDS: Record<string, string> = { 'not-recorded': 'not recorded', 'not-covered': 'not checked', excluded: 'outside this measure' };

/** Moves a label's start past any tick its box would cross, so a tick never cuts through words. */
function clearOfTicks(x: number, width: number, ticks: readonly number[]): number {
  let at = x;
  for (let pass = 0; pass < ticks.length + 1; pass++) {
    const hit = ticks.find((t) => t >= at - 2 && t <= at + width + 2);
    if (hit === undefined) break;
    at = hit + 4;
  }
  return at;
}

/**
 * A bullet row: a receding track, an optional inner base band (2px short of the amendments), a
 * thinner ink bar for the measure with an optional lighter extension, and target ticks. An absent
 * track is a short dashed stub at the baseline, never a footprint across the axis; its words join
 * the bar's words, so the two never collide.
 */
export function Bullet({ band, scale, track, base, bar, extension, ticks = [], claim = 'descriptive', barLabel, overrunLabel, trackSlotText, barSlotText, labelLimit, interaction }: BulletProps) {
  const zero = scale(Math.max(Math.min(0, Math.max(...scale.domain)), Math.min(...scale.domain)));
  const { pos, thickness } = band;
  const barThickness = Math.max(4, Math.round(thickness * 0.42));
  const barTop = pos + (thickness - barThickness) / 2;
  const paint = paintFor(claim, claim === 'statistical' ? 'outline' : 'fill', 'primary');
  const trackEnd = isValue(track) ? scale(track.value) : null;
  const barEnd = isValue(bar) ? scale(bar.value) : null;
  const extensionEnd = barEnd !== null && isValue(bar) && extension && isValue(extension) && extension.value > 0 ? scale(bar.value + extension.value) : null;
  const baseEnd = base && isValue(base) ? scale(base.value) : null;
  const over = trackEnd !== null && barEnd !== null && barEnd > trackEnd;
  const trackWords = isValue(track) ? '' : trackSlotText ?? `track ${ABSENT_WORDS[track.state]}`;
  const barWords = isValue(bar) ? '' : barSlotText ?? (bar.state === 'not-covered' ? 'not checked' : 'not recorded');
  const text = [over && overrunLabel ? overrunLabel : isValue(bar) ? barLabel : barLabel ?? barWords, trackWords].filter(Boolean).join(' · ');
  const tickXs = ticks.flatMap((tick) => (isValue(tick.at) ? [Math.round(scale(tick.at.value)) + 0.5] : []));
  const textWidth = text ? measureText(text) : 0;
  const textX = text ? clearOfTicks(Math.max(barEnd ?? zero, extensionEnd ?? zero, trackEnd ?? zero + STUB) + 6, textWidth, tickXs) : 0;
  const fitsRight = text ? textX + textWidth <= (labelLimit ?? Math.max(...scale.range)) + 2 : false;
  return (
    <MarkGroup interaction={interaction} box={{ x: zero, y: pos, width: Math.max(trackEnd ?? zero + STUB, extensionEnd ?? 0, barEnd ?? 0, zero + 1) - zero, height: thickness }}>
      {isValue(track)
        ? <rect x={zero} y={pos} width={Math.max(0, (trackEnd ?? zero) - zero)} height={thickness} rx={2} style={{ fill: VIZ.track }} />
        : <AbsentSlot absence={track} box={{ x: zero, y: pos, width: STUB, height: thickness }} text="" />}
      {baseEnd !== null && (
        // 2px of surface where the base ends and the amendments begin, so the split reads in both themes.
        <rect x={zero} y={pos} width={Math.max(0, baseEnd - zero - (trackEnd !== null && trackEnd - baseEnd > 3 ? 2 : 0))} height={thickness} rx={2} style={{ fill: 'var(--viz-ord-2)' }} />
      )}
      {extensionEnd !== null && barEnd !== null && (
        <rect x={barEnd} y={barTop} width={Math.max(0, extensionEnd - barEnd)} height={barThickness} style={{ fill: VIZ.context }} />
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
      {text && fitsRight && (
        <text x={textX} y={pos + thickness / 2} dy="0.32em" className={over ? 'viz-num fill-fg text-[11px] font-medium' : isValue(bar) ? LABEL_CLASS : MUTED_LABEL_CLASS}>{text}</text>
      )}
    </MarkGroup>
  );
}
