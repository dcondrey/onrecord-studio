import type { ReactNode } from 'react';
import type { StoryLead } from '../../types';
import { isLegalClaimKind } from '../../types/findings';
import { useAppStore } from '../../store/useAppStore';
import { cn } from '../../utils/cn';
import ScoreSpark from '../charts/figures/ScoreSpark';
import ObjectLink from '../objects/ObjectLink';
import { findingFamily } from '../objects/findingFamily';
import { claimBasisLabel, formatShortDate, severityLabel } from '../objects/labels';
import { ClaimBasisBadge } from '../ui/FamilyBadge';
import SeverityGlyph from './SeverityGlyph';
import { COUNTED_FACTOR, factorLabel, isUnassessed, orderedFactors, points } from './storyFactors';

// The headline link's ::after covers the whole row, so the row is one hit target; links inside the
// byline are raised above it and stay separately clickable.
const STRETCHED = 'text-fg no-underline decoration-fg/40 underline-offset-2 hover:underline focus-visible:underline after:absolute after:inset-0 after:rounded-lg after:content-[""]';

/**
 * The kicker over a lead's headline: its family in small capitals, then what kind of claim it is.
 * Only a counted violation takes the solid statutory badge; a pattern signal stays plain text.
 */
export function LeadKicker({ lead, prefix, className }: { lead: StoryLead; prefix?: ReactNode; className?: string }) {
  return (
    <p className={cn('flex min-w-0 flex-wrap items-center gap-x-2 gap-y-1 text-overline uppercase', className)}>
      {prefix}
      <span className="text-fg-secondary">{findingFamily(lead.kind)}</span>
      {isLegalClaimKind(lead.kind) ? (
        <ClaimBasisBadge kind={lead.kind} className="normal-case tracking-normal" />
      ) : (
        <><span aria-hidden="true" className="text-muted">·</span><span className="text-muted">{claimBasisLabel(lead.kind)}</span></>
      )}
    </p>
  );
}

/**
 * Severity, source, first-seen date and the agency the lead is about (when the analytics profile
 * names it), separated by interpuncts; nothing is printed for a part the lead lacks. `stacked` sets
 * the agency on its own line, for a column too narrow to wrap it cleanly.
 */
export function LeadByline({ lead, source, stacked = false, className }: { lead: StoryLead; source?: string; stacked?: boolean; className?: string }) {
  const { agencyId, portalKey } = lead.subject;
  const profile = useAppStore((state) => agencyId === null ? null : state.agencyProfiles.find((entry) => entry.agencyId === agencyId && entry.portalSubdomain === portalKey) ?? null);
  const agency = profile && agencyId !== null && portalKey ? (
    // Raised above a row's stretched headline link, so it stays separately clickable.
    <span key="agency" className="relative z-10 min-w-0">
      About <ObjectLink target={{ kind: 'agency', portalKey, agencyId, label: profile.agencyName }}>{profile.agencyName}</ObjectLink>
    </span>
  ) : null;
  const parts = [
    <span key="severity" className="flex items-center gap-1 text-fg-secondary"><SeverityGlyph severity={lead.severity} />{severityLabel(lead.severity)}</span>,
    source && <span key="source" className="min-w-0 truncate">{source}</span>,
    <time key="time" dateTime={lead.firstSeenAt} className="tabular-nums">{formatShortDate(lead.firstSeenAt)}</time>,
    !stacked && agency,
  ].filter(Boolean);
  const line = (
    <p className={cn('flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5 text-caption text-muted', !stacked && className)}>
      {parts.flatMap((part, index) => (index === 0 ? [part] : [<span key={`dot-${index}`} aria-hidden="true">·</span>, part]))}
    </p>
  );
  if (!stacked || !agency) return stacked ? <div className={className}>{line}</div> : line;
  return (
    <div className={cn('space-y-0.5', className)}>
      {line}
      <p className="text-caption text-muted">{agency}</p>
    </div>
  );
}

/** A lead's attention score as words, for the sparkline's accessible name. */
export function scoreLabel(lead: StoryLead): string {
  const factors = orderedFactors(lead).map((factor) => `${factorLabel(factor.name).toLowerCase()} ${isUnassessed(lead, factor.name) ? 'not assessed' : `+${points(factor.basisPoints)}`}`);
  return `Attention score ${points(lead.publicInterest.totalBasisPoints)}${factors.length ? `: ${factors.join(', ')}` : ''}`;
}

/**
 * One ranked lead as a line of the front page: a set ordinal, the kicker, the headline as the row's
 * link, the byline, and the attention score at sparkline scale on the list's shared scale.
 */
export default function LeadLine({ lead, rank, scaleMax, source }: { lead: StoryLead; rank: number; scaleMax: number; source?: string }) {
  return (
    <article className="group relative -mx-3 grid grid-cols-[2.25rem_minmax(0,1fr)] gap-x-3 rounded-lg px-3 py-4 transition-colors hover:bg-raised/60 sm:grid-cols-[2.25rem_minmax(0,1fr)_8.5rem]">
      <span aria-hidden="true" className="pt-0.5 text-rank tabular-nums text-muted transition-colors group-hover:text-fg-secondary">{rank}</span>
      <div className="min-w-0">
        <LeadKicker lead={lead} />
        <h4 className="mt-1.5 text-pretty text-heading text-fg">
          <ObjectLink target={{ kind: 'finding', id: lead.findingId, label: lead.headline }} className={STRETCHED}>{lead.headline}</ObjectLink>
        </h4>
        <LeadByline lead={lead} source={source} className="mt-1.5" />
      </div>
      <div className="col-start-2 mt-3 flex items-center gap-2 sm:col-start-3 sm:mt-0 sm:justify-end sm:self-center">
        <ScoreSpark
          scaleMax={scaleMax}
          label={scoreLabel(lead)}
          width={104}
          factors={orderedFactors(lead).map((factor) => ({ key: factor.name, basisPoints: factor.basisPoints, counted: factor.name === COUNTED_FACTOR, unassessed: isUnassessed(lead, factor.name) }))}
        />
        <span aria-hidden="true" className="w-6 text-right text-caption font-medium tabular-nums text-fg-secondary">{points(lead.publicInterest.totalBasisPoints)}</span>
      </div>
    </article>
  );
}
