import type { FindingKind } from '../../types/findings';
import { isLegalClaimKind } from '../../types/findings';
import type { LeadFamily } from '../objects/findingFamily';
import { claimBasisLabel } from '../objects/labels';
import Badge from './Badge';
import Icon, { type IconName } from './icons';

const FAMILY_ICON: Record<LeadFamily, IconName> = {
  'Records request': 'clipboard',
  Spending: 'briefcase',
  Contradiction: 'flag',
  Pattern: 'grid',
  'Data gap': 'search',
  Outlier: 'activity',
};

/** The family a lead belongs to. Violet means the app surfaced it; it never claims more than that. */
export default function FamilyBadge({ family, className }: { family: LeadFamily; className?: string }) {
  return (
    <Badge tone="lead" className={className}>
      <Icon name={FAMILY_ICON[family]} size="xs" />
      {family}
    </Badge>
  );
}

/**
 * What the finding's claim rests on. Only a kind the statute evaluator produces gets the solid
 * "Counted violation" badge; every other finding is a neutral "Pattern signal".
 */
export function ClaimBasisBadge({ kind, className }: { kind: FindingKind; className?: string }) {
  return (
    <Badge tone={isLegalClaimKind(kind) ? 'violation' : 'neutral'} className={className}>
      {claimBasisLabel(kind)}
    </Badge>
  );
}
