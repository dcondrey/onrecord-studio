import { describe, expect, it } from 'vitest';
import { describeSharedIdentifier } from '../utils/identifierEvidence';

const shared = (kind: string, displayValue: string) => ({ kind, displayValue, leftDocuments: 3, rightDocuments: 2 });

describe('describeSharedIdentifier', () => {
  it('names the kind and never claims identity from a shared identifier', () => {
    const ein = describeSharedIdentifier(shared('EIN', '12-3456789'));
    expect(ein).toContain('EIN 12-3456789');
    expect(ein).toContain('3 and 2 documents');
    expect(ein).toContain('not proof of identity');
    expect(describeSharedIdentifier(shared('CONTRACTOR_LICENSE', '123456'))).toContain('contractor license 123456');
  });

  it('says a parcel links properties only', () => {
    expect(describeSharedIdentifier(shared('PARCEL_NUMBER', '123-456-78'))).toContain('properties only');
  });

  it('keeps the email wording and falls back to an honest sentence for an unknown kind', () => {
    expect(describeSharedIdentifier(shared('EMAIL_ADDRESS', 'a@x.org'))).toContain('Co-occurrence is not ownership');
    expect(describeSharedIdentifier(shared('FUTURE', 'v'))).toContain('not proof of identity');
  });
});
