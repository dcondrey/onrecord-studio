import type { StoryLead } from '../../types';
import { isLegalClaimKind } from '../../types/findings';
import { points } from '../../utils/attentionFactors';
import ScoreSpark, { scoreSentence, sparkFactors } from '../charts/figures/ScoreSpark';
import SeverityGlyph from '../dashboard/SeverityGlyph';
import { findingFamily } from '../objects/findingFamily';
import { formatShortDate, severityLabel } from '../objects/labels';
import Badge from '../ui/Badge';
import FamilyBadge from '../ui/FamilyBadge';
import { keepIdentifiers } from './LeadMeta';

/**
 * A ranked lead, set inside the queue's button: its rank, family and (only for a counted
 * violation) the statutory badge; the headline; how serious, whose and when; then the attention
 * score as a spark on the queue's shared scale. The spark is violet because the app surfaced the
 * lead; only a counted-violation segment is crimson. The spark's numbers are read out in words.
 */
export default function LeadCard({ lead, rank, scaleMax, source }: {
  lead: StoryLead;
  rank: number;
  /** The top visible score, in basis points: every spark in the queue shares it. */
  scaleMax: number;
  /** The lead's source, named only when the queue spans more than one. */
  source?: string;
}) {
  const meta = [
    <span key="severity" className="flex shrink-0 items-center gap-1 text-fg-secondary">
      <SeverityGlyph severity={lead.severity} />
      {severityLabel(lead.severity)}
    </span>,
    source && <span key="source" className="min-w-0 truncate">{source}</span>,
    <time key="time" dateTime={lead.firstSeenAt} className="shrink-0 tabular-nums">{formatShortDate(lead.firstSeenAt)}</time>,
  ].filter(Boolean);
  return (
    <>
      <span className="flex h-5 min-w-0 items-center gap-1.5">
        <span aria-hidden="true" className="w-6 shrink-0 text-caption font-semibold tabular-nums text-fg-secondary">#{rank}</span>
        <FamilyBadge family={findingFamily(lead.kind)} compact />
        {isLegalClaimKind(lead.kind) && <Badge tone="violation">Counted</Badge>}
      </span>
      <span className="mt-1 line-clamp-2 text-body font-medium text-fg">{keepIdentifiers(lead.headline)}</span>
      <span className="flex min-w-0 items-center gap-1.5 text-caption text-muted">
        {meta.flatMap((part, index) => (index === 0 ? [part] : [<span key={`dot-${index}`} aria-hidden="true" className="shrink-0">·</span>, part]))}
      </span>
      <span className="mt-2 flex items-center gap-2">
        <ScoreSpark factors={sparkFactors(lead)} scaleMax={scaleMax} label="" decorative />
        <span aria-hidden="true" className="w-7 shrink-0 text-right text-caption font-medium tabular-nums text-fg-secondary">{points(lead.publicInterest.totalBasisPoints)}</span>
      </span>
      <span className="sr-only">{`Rank ${rank}. ${scoreSentence(lead)}`}</span>
    </>
  );
}
