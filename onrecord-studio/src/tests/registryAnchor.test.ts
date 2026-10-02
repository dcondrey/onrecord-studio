import { describe, expect, it } from 'vitest';
import { describeRegistryAnchor } from '../utils/registryAnchor';
import type { RegistryAnchor, RegistrySnapshotSummary } from '../types/registry';

const snapshot: RegistrySnapshotSummary = {
  registry: 'CSLB_LICENSE_MASTER',
  identifierKind: 'CONTRACTOR_LICENSE',
  sourcePath: '/tmp/master.csv',
  fileSha256: 'a'.repeat(64),
  importedAt: '2026-10-01T10:00:00Z',
  recordCount: 2,
  skippedRowCount: 0,
};

// The four lookup states are four facts and must read differently: an unimported registry is
// "not checked", an absent row is "not listed in this file", and a match names the number as
// registered to a name in that file without ever asserting who a party in the documents is.
describe('describeRegistryAnchor', () => {
  it('keeps absence, not-checked and registered-to distinct and never claims identity', () => {
    const states: RegistryAnchor[] = [
      { state: 'NOT_IMPORTED', registry: 'CSLB_LICENSE_MASTER' },
      { state: 'NOT_LISTED', snapshot, identifier: '123456' },
      { state: 'REGISTERED', snapshot, identifier: '123456', record: { registrantName: 'ACME INC', status: 'ACTIVE', detail: '' } },
    ];
    const [notImported, notListed, registered] = states.map(describeRegistryAnchor);
    expect(notImported).toMatch(/^Not checked/);
    expect(notImported).not.toContain('sha256');
    expect(notListed).toMatch(/^Not listed/);
    expect(notListed).toContain('sha256:aaaaaaaaaaaaaaaa');
    expect(notListed).toContain('not that it was never issued');
    expect(registered).toContain('registered to ACME INC (ACTIVE)');
    expect(registered).toContain('sha256:aaaaaaaaaaaaaaaa');
    expect(registered).toContain('does not say that a party here is that registrant');
    expect(registered).not.toMatch(/\bis ACME\b/);
  });
});
