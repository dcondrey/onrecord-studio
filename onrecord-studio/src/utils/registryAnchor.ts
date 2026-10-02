import type { RegistryAnchor, RegistryKind, RegistrySnapshotSummary } from '../types/registry';

export const REGISTRY_TITLES: Record<RegistryKind, string> = {
  CSLB_LICENSE_MASTER: 'CSLB License Master list',
};

export function describeSnapshot(snapshot: RegistrySnapshotSummary): string {
  return `${REGISTRY_TITLES[snapshot.registry]} imported ${snapshot.importedAt.slice(0, 10)}, file sha256:${snapshot.fileSha256.slice(0, 16)}, ${snapshot.recordCount.toLocaleString('en-US')} records`;
}

/**
 * One sentence per lookup state. A registered match grades as "the number is registered to
 * NAME": it is a statement about the number in a named file, not about any party in the documents.
 */
export function describeRegistryAnchor(anchor: RegistryAnchor): string {
  switch (anchor.state) {
    case 'NO_REGISTRY':
      return 'No public registry is configured for this identifier kind; it was not checked.';
    case 'NOT_IMPORTED':
      return `Not checked: no ${REGISTRY_TITLES[anchor.registry]} snapshot has been imported.`;
    case 'NOT_LISTED':
      return `Not listed in the ${describeSnapshot(anchor.snapshot)}. The list holds licences currently renewed or expired but renewable, so this says the number is absent from that file, not that it was never issued.`;
    case 'REGISTERED':
      return `Number ${anchor.identifier} is registered to ${anchor.record.registrantName}${anchor.record.status ? ` (${anchor.record.status})` : ''} in the ${describeSnapshot(anchor.snapshot)}. This anchors the number, not any name in the documents: it does not say that a party here is that registrant.`;
  }
}
