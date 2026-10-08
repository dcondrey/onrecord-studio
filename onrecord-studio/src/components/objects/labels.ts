import type { FindingConfidence, FindingKind, FindingSeverity } from '../../types';
import { isLegalClaimKind } from '../../types/findings';

/** `SCREAMING_SNAKE` enum value as a sentence-case phrase. */
export function enumLabel(value: string): string {
  if (value === 'CO_OCCURRENCE') return 'Co-occurrence';
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
  // A bare calendar date parses as UTC midnight; read it in UTC so it is not shown as the day before.
  const dateOnly = value.length === 10 && !value.includes('T');
  return Number.isNaN(date.valueOf())
    ? value
    : new Intl.DateTimeFormat(undefined, { month: 'short', day: 'numeric', year: 'numeric', timeZone: dateOnly ? 'UTC' : undefined }).format(date);
}

/**
 * An instant in the reader's zone, to the minute, with the zone named: "Oct 7, 2026, 8:00 AM PDT".
 * Callers keep the raw ISO string on `dateTime` or the tooltip, which is where the exact second
 * lives. A value that will not parse is shown as recorded rather than as "Invalid Date", which
 * would hide a malformed timestamp.
 */
export function formatInstant(value: string): string {
  const parsed = new Date(value);
  if (Number.isNaN(parsed.getTime())) return value;
  return parsed.toLocaleString(undefined, {
    year: 'numeric',
    month: 'short',
    day: 'numeric',
    hour: 'numeric',
    minute: '2-digit',
    timeZoneName: 'short',
  });
}

/**
 * What one provenance check established. A check that hashed nothing, or had no recorded digest
 * to compare against, says so; it is never reported as a mismatch.
 */
export function provenanceVerdict(check: { evidenceFilePresent: boolean | null; expectedSha256: string | null; hashMatches: boolean; verifiedAt?: string }): { tone: 'success' | 'danger' | 'muted'; text: string } {
  const when = check.verifiedAt ? ` (${formatShortDate(check.verifiedAt)})` : '';
  if (check.evidenceFilePresent === false) return { tone: 'muted', text: `No stored file to check; nothing was hashed${when}` };
  if (!check.expectedSha256) return { tone: 'muted', text: `No digest was recorded, so the file could not be compared${when}` };
  return check.hashMatches
    ? { tone: 'success', text: `File matched its recorded digest${when}` }
    : { tone: 'danger', text: `File did not match its recorded digest${when}` };
}
