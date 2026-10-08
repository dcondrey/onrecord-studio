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
