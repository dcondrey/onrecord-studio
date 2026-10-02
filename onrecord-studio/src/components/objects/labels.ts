import type { FindingConfidence, FindingKind, FindingSeverity } from '../../types';
import { isLegalClaimKind } from '../../types/findings';

/** `SCREAMING_SNAKE` enum value as a sentence-case phrase. */
export function enumLabel(value: string): string {
  const words = value.toLowerCase().split('_').join(' ');
  return words.charAt(0).toUpperCase() + words.slice(1);
}

export function findingKindLabel(kind: FindingKind): string {
  return enumLabel(kind);
}

export function severityLabel(severity: FindingSeverity): string {
  return severity.charAt(0) + severity.slice(1).toLowerCase();
}

export function severityTone(severity: FindingSeverity): 'danger' | 'warning' | 'accent' | 'muted' {
  switch (severity) {
    case 'CRITICAL': return 'danger';
    case 'SERIOUS': return 'warning';
    case 'NOTABLE': return 'accent';
    case 'INFO': return 'muted';
  }
}

/** "Counted violation" only for a kind the statute evaluator produces; everything else is a signal. */
export function claimBasisLabel(kind: FindingKind): string {
  return isLegalClaimKind(kind) ? 'Counted violation' : 'Pattern signal';
}

export function confidenceLabel(confidence: FindingConfidence): string {
  return confidence.kind === 'MEASURED'
    ? `${(confidence.basisPoints / 100).toFixed(0)}% measured confidence`
    : 'Confidence not assessed';
}

export function formatShortDate(value: string): string {
  const date = new Date(value);
  return Number.isNaN(date.valueOf())
    ? value
    : new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', year: 'numeric' }).format(date);
}
