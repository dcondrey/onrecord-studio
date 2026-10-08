import type { StoryLead } from '../../types';

/** The seven factors the ranker can emit (analysis/story_desk.rs), in every figure's fixed order. */
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

export const isKnownFactor = (name: string) => (FACTOR_ORDER as readonly string[]).includes(name);

// The backend names factors in lowercase prose; figures show one consistent label per factor.
const FACTOR_LABELS: Record<string, string> = {
  'money at stake': 'Money',
  scale: 'Scale',
  recurrence: 'Recurrence',
  'record contradicts statement': 'Contradiction',
  'counted statutory or policy violation': 'Counted violation',
  'evidence strength': 'Evidence strength',
  freshness: 'Freshness',
};

export function factorLabel(name: string) {
  return FACTOR_LABELS[name] ?? name.charAt(0).toUpperCase() + name.slice(1);
}

export const points = (basisPoints: number) => Math.round(basisPoints / 100);
export const factorPoints = (lead: StoryLead, name: string) => lead.publicInterest.factors.find((factor) => factor.name === name)?.basisPoints ?? 0;

/** Evidence strength on a finding whose confidence was never assessed: not a measured zero. */
export const isUnassessed = (lead: StoryLead, name: string) => name === EVIDENCE_FACTOR && lead.confidence.kind === 'UNASSESSED';

/** A lead's factors in the fixed order; a name outside the closed set follows, never dropped. */
export function orderedFactors(lead: StoryLead) {
  const rank = (name: string) => (isKnownFactor(name) ? FACTOR_ORDER.indexOf(name as (typeof FACTOR_ORDER)[number]) : FACTOR_ORDER.length);
  return [...lead.publicInterest.factors].sort((a, b) => rank(a.name) - rank(b.name));
}

/**
 * The deterministic headline: what lifts #1, and on which factor it is ahead of #2. Factor names
 * are the ranker's own lowercase prose.
 */
export function storyHeadline(leads: readonly StoryLead[]): string {
  const [first, second] = leads;
  if (!first) return '';
  const top = [...first.publicInterest.factors].filter((factor) => factor.basisPoints > 0).sort((a, b) => b.basisPoints - a.basisPoints).slice(0, 2);
  const lead = `#1 scores ${points(first.publicInterest.totalBasisPoints)}`;
  const from = top.length === 0 ? '' : `: most from ${top.map((factor) => `${factor.name} (+${points(factor.basisPoints)})`).join(' and ')}`;
  if (!second) return `${lead}${from}.`;
  const delta = points(first.publicInterest.totalBasisPoints) - points(second.publicInterest.totalBasisPoints);
  if (delta === 0) return `${lead}${from}. It ties #2.`;
  const names = [...new Set([...first.publicInterest.factors, ...second.publicInterest.factors].map((factor) => factor.name))];
  const lead1Ahead = (name: string) => factorPoints(first, name) - factorPoints(second, name);
  const gap = names.reduce((best, name) => (lead1Ahead(name) > lead1Ahead(best) ? name : best), names[0]);
  return `${lead}${from}. It is ahead of #2 by ${delta}, mostly on ${gap}.`;
}
