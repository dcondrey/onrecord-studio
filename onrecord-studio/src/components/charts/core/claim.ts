/**
 * The claim contract. A mark states what kind of claim it carries, and its paint follows from that
 * alone: only a counted legal or policy measure (from the statute or policy evaluator) may be
 * crimson, and a statistical comparison can only be an ink outline, a hollow mark or a dashed line,
 * so a yardstick can never look like a finding.
 */

export type Claim = 'counted' | 'statistical' | 'descriptive';

/** A descriptive mark's role. Ignored for counted and statistical claims. */
export type MarkRole = 'primary' | 'default' | 'comparison' | 'lead' | 'positive';

export type MarkVariant = 'fill' | 'outline' | 'hollow' | 'line';

export interface Paint {
  fill: string;
  stroke: string;
  strokeWidth: number;
  dash?: string;
}

export const VIZ = {
  surface: 'var(--viz-surface)',
  ink: 'var(--viz-ink)',
  mark: 'var(--viz-mark)',
  context: 'var(--viz-context)',
  track: 'var(--viz-track)',
  grid: 'var(--viz-grid)',
  baseline: 'var(--viz-baseline)',
  lead: 'var(--viz-lead)',
  positive: 'var(--viz-positive)',
  counted: 'var(--viz-counted)',
  focus: 'var(--viz-focus)',
} as const;

const ROLE_COLOUR: Record<MarkRole, string> = {
  primary: VIZ.ink,
  default: VIZ.mark,
  comparison: VIZ.context,
  lead: VIZ.lead,
  positive: VIZ.positive,
};

export const COMPARISON_DASH = '4 3';
export const STATISTICAL_DASH = '6 3';

export function paintFor(claim: Claim, variant: MarkVariant, role: MarkRole = 'default'): Paint {
  if (claim === 'counted') {
    if (variant === 'fill') return { fill: VIZ.counted, stroke: 'none', strokeWidth: 0 };
    if (variant === 'line') return { fill: 'none', stroke: VIZ.counted, strokeWidth: 2 };
    return { fill: variant === 'hollow' ? VIZ.surface : 'none', stroke: VIZ.counted, strokeWidth: 1.5 };
  }
  if (claim === 'statistical') {
    // No accent and no solid fill, whatever was asked for.
    if (variant === 'line') return { fill: 'none', stroke: VIZ.ink, strokeWidth: 2, dash: STATISTICAL_DASH };
    return { fill: variant === 'hollow' ? VIZ.surface : 'none', stroke: VIZ.ink, strokeWidth: 1.5 };
  }
  const colour = ROLE_COLOUR[role];
  if (variant === 'line') return { fill: 'none', stroke: colour, strokeWidth: 2, dash: role === 'comparison' ? COMPARISON_DASH : undefined };
  if (variant === 'fill' && role !== 'comparison') return { fill: colour, stroke: 'none', strokeWidth: 0 };
  if (variant === 'outline') return { fill: 'none', stroke: colour, strokeWidth: 1.5 };
  // A comparison dot is hollow; a hollow mark keeps the surface inside so lines do not show through.
  return { fill: VIZ.surface, stroke: colour, strokeWidth: 1.5 };
}

/** Header tag text for a frame's claim; descriptive figures carry none. */
export const CLAIM_TAG: Record<Exclude<Claim, 'descriptive'>, string> = {
  counted: 'Counted (legal claim)',
  statistical: 'Statistical comparison, not a legal finding',
};

// ---- Reference lines -------------------------------------------------------------------------

/**
 * - statute: a line the engine evaluated, with its citation; ink 1px solid.
 * - yardstick: an app reference that is not a rule; muted and dashed, and its label says so.
 * - peer-mean: the mean of comparable peers; context 1px solid.
 * - today: the current date; mark 1px solid.
 * None of them is ever crimson.
 */
export type ReferenceKind = 'statute' | 'yardstick' | 'peer-mean' | 'today';

/**
 * A yardstick's label must say it is a reference or not a rule ('not a rule', 'not a budget rule'),
 * so it cannot be read as one.
 */
export type YardstickLabel = `${string}reference${string}` | `Reference${string}` | `${string}not a ${string}rule${string}`;

export function assertYardstickLabel(label: string): asserts label is YardstickLabel {
  if (!/[Rr]eference|not a (?:[\w-]+ )*rule/.test(label)) {
    throw new Error(`A yardstick label must say 'reference' or 'not a rule': "${label}"`);
  }
}

export const REFERENCE_PAINT: Record<ReferenceKind, Paint> = {
  statute: { fill: 'none', stroke: VIZ.ink, strokeWidth: 1 },
  yardstick: { fill: 'none', stroke: VIZ.context, strokeWidth: 1, dash: COMPARISON_DASH },
  'peer-mean': { fill: 'none', stroke: VIZ.context, strokeWidth: 1 },
  today: { fill: 'none', stroke: VIZ.mark, strokeWidth: 1 },
};
