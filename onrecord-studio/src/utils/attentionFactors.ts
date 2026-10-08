import type { StoryLead } from '../types';

/** The seven factors the ranker can emit (analysis/story_desk.rs), in the figures' fixed order. */
export const FACTOR_ORDER = [
  'money at stake',
  'scale',
  'recurrence',
  'record contradicts statement',
  'counted statutory or policy violation',
  'evidence strength',
  'freshness',
] as const;

export const COUNTED_FACTOR = 'counted statutory or policy violation';
export const EVIDENCE_FACTOR = 'evidence strength';
/** The largest single factor (money at stake), so every factor shares one 0–2500 bp scale. */
export const FACTOR_SCALE_MAX_BP = 2500;

// The backend names factors in lowercase prose; the figures show one consistent label per factor.
const FACTOR_LABELS: Record<string, string> = {
  'money at stake': 'Money',
  scale: 'Scale',
  recurrence: 'Recurrence',
  'record contradicts statement': 'Contradiction',
  'counted statutory or policy violation': 'Counted violation',
  'evidence strength': 'Evidence strength',
  freshness: 'Freshness',
};

/** Labels for a frame too narrow for the full ones; the full wording stays in the title. */
export const FACTOR_SHORT: Record<string, string> = {
  'money at stake': 'Money',
  scale: 'Scale',
  recurrence: 'Recur.',
  'record contradicts statement': 'Contra.',
  'counted statutory or policy violation': 'Counted',
  'evidence strength': 'Evid.',
  freshness: 'Fresh',
};

export const isKnownFactor = (name: string) => (FACTOR_ORDER as readonly string[]).includes(name);

export function factorLabel(name: string) {
  return FACTOR_LABELS[name] ?? name.charAt(0).toUpperCase() + name.slice(1);
}

export const points = (basisPoints: number) => Math.round(basisPoints / 100);

export const factorPoints = (lead: StoryLead, name: string) => lead.publicInterest.factors.find((factor) => factor.name === name)?.basisPoints ?? 0;

/** Evidence strength on a finding whose confidence was never assessed: not a measured zero. */
export const isUnassessed = (lead: StoryLead, name: string) => name === EVIDENCE_FACTOR && lead.confidence.kind === 'UNASSESSED';

/** A lead's emitted factors in the fixed order; a name outside the closed set follows, never dropped. */
export function orderedFactors(lead: StoryLead) {
  const at = (name: string) => (isKnownFactor(name) ? FACTOR_ORDER.indexOf(name as (typeof FACTOR_ORDER)[number]) : FACTOR_ORDER.length);
  return [...lead.publicInterest.factors].sort((a, b) => at(a.name) - at(b.name));
}

/** Basis points a factor gave a lead; null when its record lacks the factor or never assessed it. */
export function measuredFactor(lead: StoryLead, name: string): number | null {
  const factor = lead.publicInterest.factors.find((candidate) => candidate.name === name);
  return !factor || isUnassessed(lead, name) ? null : factor.basisPoints;
}

/**
 * The deterministic headline over a lead's factors: its score, the factors that carry most of it,
 * and how it stands against the lead it is compared with. Factor names are the ranker's own
 * lowercase prose. "Mostly on" names the factor with the largest gap in the direction of the
 * difference; a factor that either lead lacks or never assessed adds nothing to that gap.
 */
export function comparisonHeadline(lead: StoryLead, rank: number, compare?: { lead: StoryLead; rank: number }): string {
  const score = points(lead.publicInterest.totalBasisPoints);
  const top = lead.publicInterest.factors
    .filter((factor) => factor.basisPoints > 0 && !isUnassessed(lead, factor.name))
    .sort((a, b) => b.basisPoints - a.basisPoints)
    .slice(0, 2);
  const opening = top.length === 0
    ? `#${rank} scores ${score}; no factor added points.`
    : `#${rank} scores ${score}, most from ${top.map((factor) => `${factor.name} (+${points(factor.basisPoints)})`).join(' and ')}.`;
  if (!compare) return opening;
  const delta = score - points(compare.lead.publicInterest.totalBasisPoints);
  if (delta === 0) return `${opening} It ties #${compare.rank}.`;
  const direction = Math.sign(delta);
  const names = [...new Set([...lead.publicInterest.factors, ...compare.lead.publicInterest.factors].map((factor) => factor.name))];
  let widest: { name: string; gap: number } | null = null;
  for (const name of names) {
    const gap = direction * ((measuredFactor(lead, name) ?? 0) - (measuredFactor(compare.lead, name) ?? 0));
    if (gap > 0 && (!widest || gap > widest.gap)) widest = { name, gap };
  }
  return `${opening} It ${delta > 0 ? 'is ahead of' : 'trails'} #${compare.rank} by ${Math.abs(delta)}${widest ? `, mostly on ${widest.name}` : ''}.`;
}

/** The lead a ranked lead is compared with: #2 for #1, otherwise the lead just above it. */
export function comparisonOf<T>(leads: readonly T[], index: number): { lead: T; rank: number } | undefined {
  const at = index === 0 ? 1 : index - 1;
  return index >= 0 && leads[at] !== undefined ? { lead: leads[at], rank: at + 1 } : undefined;
}
