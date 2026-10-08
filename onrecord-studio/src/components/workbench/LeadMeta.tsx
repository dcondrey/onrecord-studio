import type { ReactNode } from 'react';
import { useAppStore } from '../../store/useAppStore';
import type { FindingKind, FindingSeverity } from '../../types';
import { isLegalClaimKind } from '../../types/findings';
import { cn } from '../../utils/cn';
import SeverityGlyph from '../dashboard/SeverityGlyph';
import { findingFamily } from '../objects/findingFamily';
import { formatShortDate, severityLabel } from '../objects/labels';
import ObjectLink from '../objects/ObjectLink';
import { ClaimBasisBadge } from '../ui/FamilyBadge';

const isDigit = (char: string) => char >= '0' && char <= '9';

/**
 * Sets each identifier such as "H-24-1180" unbroken: a line break at its hyphens reads as two
 * numbers. A word is an identifier when it carries both a hyphen and a digit.
 */
export function keepIdentifiers(text: string): ReactNode[] {
  const parts: ReactNode[] = [];
  let run = '';
  text.split(' ').forEach((word, index) => {
    run += index === 0 ? '' : ' ';
    if (word.includes('-') && [...word].some(isDigit)) {
      if (run) parts.push(run);
      run = '';
      parts.push(<span key={index} className="whitespace-nowrap">{word}</span>);
    } else {
      run += word;
    }
  });
  if (run) parts.push(run);
  return parts;
}

/**
 * The kicker over a lead's headline: its family in small capitals, what kind of claim it is, then
 * its severity. Only a counted violation takes the solid statutory badge; a pattern signal stays
 * plain text.
 */
export function LeadKicker({ kind, severity, className }: { kind: FindingKind; severity: FindingSeverity; className?: string }) {
  return (
    <p className={cn('flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1', className)}>
      <span className="text-overline uppercase text-fg-secondary">{findingFamily(kind)}</span>
      {isLegalClaimKind(kind) ? (
        <ClaimBasisBadge kind={kind} />
      ) : (
        <><span aria-hidden="true" className="text-overline text-muted">·</span><span className="text-overline uppercase text-muted">Pattern signal</span></>
      )}
      <span aria-hidden="true" className="text-overline text-muted">·</span>
      <span className="inline-flex items-center gap-1 text-caption text-fg-secondary">
        <SeverityGlyph severity={severity} />
        {severityLabel(severity)}
      </span>
    </p>
  );
}

/**
 * Source, first-seen date and the agency the lead is about (when the analytics profile names it),
 * separated by interpuncts that wrap with the part they precede; nothing is printed for a part the
 * lead lacks.
 */
export function LeadByline({ source, firstSeen, portalKey, agencyId, className }: {
  source?: string;
  firstSeen: string;
  portalKey: string | null;
  agencyId: number | null;
  className?: string;
}) {
  const profile = useAppStore((state) => agencyId === null ? null : state.agencyProfiles.find((entry) => entry.agencyId === agencyId && entry.portalSubdomain === portalKey) ?? null);
  const parts = [
    source && { key: 'source', content: source },
    { key: 'time', content: <>First seen <time dateTime={firstSeen} className="tabular-nums">{formatShortDate(firstSeen)}</time></> },
    profile && agencyId !== null && portalKey && {
      key: 'agency',
      content: <>About <ObjectLink target={{ kind: 'agency', portalKey, agencyId, label: profile.agencyName }}>{profile.agencyName}</ObjectLink></>,
    },
  ].filter((part) => !!part);
  return (
    <p className={cn('flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-caption text-muted', className)}>
      {parts.map((part, index) => (
        <span key={part.key}>{index > 0 && <span aria-hidden="true" className="mr-1.5">·</span>}{part.content}</span>
      ))}
    </p>
  );
}
