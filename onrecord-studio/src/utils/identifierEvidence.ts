import type { SharedIdentifier } from '../types/resolution';

const DISCLAIMER = 'A shared identifier, not proof of identity; read the documents.';

/** What a shared identifier does and does not show, per kind. Never stronger than the evidence. */
export function describeSharedIdentifier(shared: SharedIdentifier): string {
  const documents = `in ${shared.leftDocuments} and ${shared.rightDocuments} documents`;
  switch (shared.kind) {
    case 'EMAIL_ADDRESS':
      return `Both appear in the same text segment as ${shared.displayValue} (${shared.leftDocuments} and ${shared.rightDocuments} documents). Co-occurrence is not ownership; read the documents.`;
    case 'EIN':
      return `Both appear beside EIN ${shared.displayValue} ${documents}. ${DISCLAIMER}`;
    case 'CONTRACTOR_LICENSE':
      return `Both appear beside contractor license ${shared.displayValue} ${documents}. ${DISCLAIMER}`;
    case 'PARCEL_NUMBER':
      return `Same parcel ${shared.displayValue}: properties only. A parcel is not evidence that two people or organizations are the same.`;
    default:
      return `Both appear beside ${shared.displayValue} ${documents}. ${DISCLAIMER}`;
  }
}
