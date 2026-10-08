import type { StoryLead } from '../../types';
import { isLegalClaimKind } from '../../types/findings';
import { points } from '../../utils/attentionFactors';
import ScoreSpark, { scoreSentence, sparkFactors } from '../charts/figures/ScoreSpark';
import SeverityGlyph from '../dashboard/SeverityGlyph';
import { findingFamily } from '../objects/findingFamily';
import { severityLabel } from '../objects/labels';
import Badge from '../ui/Badge';
import FamilyBadge from '../ui/FamilyBadge';
import { keepIdentifiers } from './LeadMeta';

/**
 * A ranked lead, set inside the queue's button in three rows so four fit a 700px window: its rank,
 * family and the statutory badge (only for a counted violation); the headline; then how serious and
 * whose, with the attention score as a spark on the queue's shared scale and in points. When it was
 * first seen is the stage's byline: the queue is ordered by score, inside the Since window. The
 * spark is violet because the app surfaced the lead; only a counted-violation segment is crimson.
 * The spark's numbers are read out in words.
 */
export default function LeadCard({ lead, rank, scaleMax, source }: {
  lead: StoryLead;
  rank: number;
  /** The top visible score, in basis points: every spark in the queue shares it. */
  scaleMax: number;
  /** The lead's source, named only when the queue spans more than one. */
  source?: string;
}) {
  return (
    <>
      <span className="flex h-5 min-w-0 items-center gap-1.5">
        <span aria-hidden="true" className="w-6 shrink-0 text-caption font-semibold tabular-nums text-fg-secondary">#{rank}</span>
        <FamilyBadge family={findingFamily(lead.kind)} compact />
        {isLegalClaimKind(lead.kind) && <Badge tone="violation">Counted</Badge>}
      </span>
      <span className="mt-0.5 line-clamp-2 text-body font-medium text-fg">{keepIdentifiers(lead.headline)}</span>
      <span className="mt-1 flex h-4 min-w-0 items-center gap-1.5 text-caption text-muted">
        <span className="flex shrink-0 items-center gap-1 text-fg-secondary">
          <SeverityGlyph severity={lead.severity} />
          {severityLabel(lead.severity)}
        </span>
        {source && <span className="min-w-0 truncate" title={source}><span aria-hidden="true">· </span>{source}</span>}
        <span className="ml-1.5 flex min-w-16 flex-1"><ScoreSpark factors={sparkFactors(lead)} scaleMax={scaleMax} label="" decorative /></span>
        <span aria-hidden="true" className="w-6 shrink-0 text-right font-medium tabular-nums text-fg-secondary">{points(lead.publicInterest.totalBasisPoints)}</span>
      </span>
      <span className="sr-only">{`Rank ${rank}. ${scoreSentence(lead)}`}</span>
    </>
  );
}
