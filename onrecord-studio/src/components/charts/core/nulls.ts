/**
 * The gap model. Every plotted value is `Measured`, so a bare 0 cannot stand in for absence: a
 * measured zero is drawn as a zero-length mark labelled '0', and each kind of absence is drawn as
 * its own kind of gap, never as a value.
 */

export type AbsenceState = 'not-recorded' | 'not-covered' | 'excluded';

/**
 * - not-recorded: the source has no value (a null average, an unpublished year).
 * - not-covered: never checked or collected (a year not checked, a day outside the file).
 * - excluded: real records outside this measure's denominator; drawn beside a bar, never on an axis.
 */
export interface Absence {
  state: AbsenceState;
  reason: string;
}

export type Measured<T = number> = { state: 'value'; value: T } | Absence;

export function measured<T>(value: T): Measured<T> {
  return { state: 'value', value };
}

export function absent(state: AbsenceState, reason: string): Absence {
  return { state, reason };
}

export function isValue<T>(m: Measured<T>): m is { state: 'value'; value: T } {
  return m.state === 'value';
}

export function isAbsence(x: unknown): x is Absence {
  return typeof x === 'object' && x !== null && 'state' in x && 'reason' in x
    && ((x as Absence).state === 'not-recorded' || (x as Absence).state === 'not-covered' || (x as Absence).state === 'excluded');
}

/** A nullable source value as `Measured`: null or undefined becomes the stated absence, never 0. */
export function fromNullable<T>(v: T | null | undefined, state: AbsenceState, reason: string): Measured<T> {
  return v === null || v === undefined ? { state, reason } : { state: 'value', value: v };
}

/**
 * Splits a series into runs of consecutive defined points. A run never spans an undefined point,
 * so a line or area drawn per run cannot interpolate across a gap.
 */
export function segmentsOf<P>(points: readonly P[], defined: (p: P) => boolean): P[][] {
  const runs: P[][] = [];
  let run: P[] = [];
  for (const point of points) {
    if (defined(point)) {
      run.push(point);
    } else if (run.length > 0) {
      runs.push(run);
      run = [];
    }
  }
  if (run.length > 0) runs.push(run);
  return runs;
}

export const ABSENCE_LABEL: Record<AbsenceState, string> = {
  'not-recorded': 'No value recorded',
  'not-covered': 'Not checked',
  excluded: 'Outside this measure',
};

export type AbsenceSwatch = 'slot-dashed' | 'slot-dotted' | 'hatch';

export const ABSENCE_SWATCH: Record<AbsenceState, AbsenceSwatch> = {
  'not-recorded': 'slot-dashed',
  'not-covered': 'slot-dotted',
  excluded: 'hatch',
};

const ORDER: readonly AbsenceState[] = ['not-recorded', 'not-covered', 'excluded'];

/** One legend entry per absence state present, in a fixed order. */
export function absenceLegend(states: Iterable<AbsenceState>): { key: string; label: string; swatch: AbsenceSwatch }[] {
  const present = new Set(states);
  return ORDER.filter((state) => present.has(state)).map((state) => ({ key: `absence-${state}`, label: ABSENCE_LABEL[state], swatch: ABSENCE_SWATCH[state] }));
}
