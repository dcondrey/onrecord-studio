import { describe, expect, it } from 'vitest';

import { compileToLegacyQuery, parseQualifierQuery } from '../query';

const agencies = [
  { agencyId: 10, agencyName: 'City Attorney', portalKey: 'oakland' },
  { agencyId: 20, agencyName: 'Public Works', portalKey: 'oakland' },
  { agencyId: 30, agencyName: 'Public Works', portalKey: 'berkeley' },
];

describe('qualifier query compilation', () => {
  it('separates searchable text and applies the flat qualifiers the native command supports', () => {
    const compiled = compileToLegacyQuery(
      parseQualifierQuery('"body camera" agency:"City Attorney" status:closed flag:late portal:oakland'),
      agencies,
    );

    expect(compiled).toEqual({
      searchTerm: 'body camera',
      portalKey: 'oakland',
      agencyId: 10,
      status: 'closed',
      policyDeviation: 'LATE',
      deferredFilters: [],
    });
  });

  it('defers unsupported, negated, ambiguous, and duplicate qualifiers instead of dropping them', () => {
    const compiled = compileToLegacyQuery(
      parseQualifierQuery('records agency:"Public Works" status:open status:closed -flag:late after:2025-01-01'),
      agencies,
    );

    expect(compiled.searchTerm).toBe('records');
    expect(compiled.agencyId).toBeNull();
    expect(compiled.status).toBeNull();
    expect(compiled.policyDeviation).toBeNull();
    expect(compiled.deferredFilters.map((filter) => filter.raw)).toEqual([
      'agency:"Public Works"',
      'status:open',
      'status:closed',
      '-flag:late',
      'after:2025-01-01',
    ]);
  });

  it('warns about an unknown qualifier without turning it into document text', () => {
    const parsed = parseQualifierQuery('budget mystery:value');
    expect(parsed.text).toBe('budget');
    expect(parsed.issues).toEqual([
      expect.objectContaining({ severity: 'warning', token: 'mystery:value' }),
    ]);
  });
});
