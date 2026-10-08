// @vitest-environment jsdom
//
// Why a lead ranks where it does: a factor the finding's record lacks is said in words and never
// drawn as a bar, an unassessed evidence strength is the not-recorded slot rather than a zero,
// and only the counted-violation factor carries the counted (crimson) paint.
import { cleanup, render } from '@testing-library/react';
import { afterEach, describe, expect, it } from 'vitest';
import AttentionFactors from '../components/charts/figures/AttentionFactors';
import type { StoryLead } from '../types';

afterEach(cleanup);

const lead = {
  findingId: 'f-1',
  kind: 'POLICY_VIOLATION_DETECTED',
  headline: 'Request H-24-1180 answered past its statutory deadline',
  severity: 'SERIOUS',
  confidence: { kind: 'UNASSESSED' },
  subject: { portalKey: 'sd-city', agencyId: null, requestId: 7, documentId: null, entityId: null },
  portalKey: 'sd-city',
  publicInterest: {
    totalBasisPoints: 4300,
    factors: [
      { name: 'money at stake', basisPoints: 2500, evidence: 'largest figure named: $1,486,200' },
      { name: 'counted statutory or policy violation', basisPoints: 1800, evidence: 'counted' },
      { name: 'evidence strength', basisPoints: 0, evidence: 'confidence not assessed' },
    ],
  },
  relatedFindingIds: [],
  firstSeenAt: '2026-10-07T00:00:00Z',
  lastSeenAt: '2026-10-07T00:00:00Z',
} as unknown as StoryLead;

describe('AttentionFactors', () => {
  it('says an absent factor in words, slots an unassessed one, and paints only the counted factor crimson', () => {
    const { container } = render(<AttentionFactors lead={lead} rank={1} />);
    const mark = (factor: string) => container.querySelector(`g[aria-label^="${factor}:"]`);

    const scale = mark('Scale');
    expect(scale?.textContent).toBe('not in this finding’s record');
    expect(scale?.querySelector('path')).toBeNull();

    const evidence = mark('Evidence strength');
    expect(evidence?.querySelector('[data-absence="not-recorded"]')).toBeTruthy();
    expect(evidence?.querySelector('path')).toBeNull();

    const counted = [...container.querySelectorAll('path')].filter((path) => path.style.fill === 'var(--viz-counted)');
    expect(counted).toHaveLength(1);
    expect(mark('Counted violation')?.contains(counted[0])).toBe(true);
    expect(mark('Money')?.querySelector('path')?.style.fill).toBe('var(--viz-lead)');
  });
});
