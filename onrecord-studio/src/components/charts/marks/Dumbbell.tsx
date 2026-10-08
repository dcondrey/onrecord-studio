import { VIZ, type Claim } from '../core/claim';
import { ABSENCE_LABEL, isValue, type Measured } from '../core/nulls';
import type { ContinuousScale } from '../core/scale';
import { Dot } from './Dot';
import { LABEL_CLASS, MUTED_LABEL_CLASS, MarkGroup, type MarkInteraction } from './shared';

export interface DumbbellProps {
  y: number;
  scale: ContinuousScale;
  /** The prior period or comparison group: hollow. */
  from: Measured;
  /** The current period or subject: solid. */
  to: Measured;
  claim?: Claim;
  /** The signed change, printed after the pair ('+0.2 pts'). */
  deltaLabel?: string;
  interaction?: MarkInteraction;
}

/**
 * A paired comparison on one axis: hollow for the prior or comparison value, solid for the current
 * or subject, joined by a context connector. A side with no value draws no dot and says so.
 */
export function Dumbbell({ y, scale, from, to, claim = 'descriptive', deltaLabel, interaction }: DumbbellProps) {
  const fx = isValue(from) ? scale(from.value) : null;
  const tx = isValue(to) ? scale(to.value) : null;
  const xs = [fx, tx].filter((v): v is number => v !== null);
  const right = Math.max(...xs, Math.min(...scale.range));
  const missing = !isValue(from) ? from : !isValue(to) ? to : null;
  return (
    <MarkGroup interaction={interaction} box={{ x: Math.min(...xs, right) - 4, y: y - 4, width: Math.abs(right - Math.min(...xs, right)) + 8, height: 8 }}>
      {fx !== null && tx !== null && Math.abs(tx - fx) > 8 && (
        <line x1={fx + Math.sign(tx - fx) * 5} x2={tx - Math.sign(tx - fx) * 5} y1={y} y2={y} style={{ stroke: VIZ.context, strokeWidth: 2, strokeLinecap: 'round' }} />
      )}
      {fx !== null && <Dot cx={fx} cy={y} claim={claim} role="comparison" hollow />}
      {tx !== null && <Dot cx={tx} cy={y} claim={claim} role="primary" />}
      {missing ? (
        <text x={right + 10} y={y} dy="0.32em" className={MUTED_LABEL_CLASS}>
          <title>{`${ABSENCE_LABEL[missing.state]}: ${missing.reason}`}</title>
          {!isValue(from) ? 'prior not recorded' : 'current not recorded'}
        </text>
      ) : deltaLabel ? (
        <text x={right + 10} y={y} dy="0.32em" className={LABEL_CLASS}>{deltaLabel}</text>
      ) : null}
    </MarkGroup>
  );
}
