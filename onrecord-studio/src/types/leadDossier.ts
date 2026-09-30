import type { Finding, FindingConfidence, FindingKind, FindingSeverity } from './findings';

/** One cited span resolved to its record. `fileName` is null when the document could not be resolved. */
export interface EvidenceLink {
  portalKey: string;
  documentId: number;
  fileName: string | null;
  requestId: number | null;
  agencyLabel: string | null;
  rawFileSha256: string | null;
  characterStart: number | null;
  characterEnd: number | null;
  quote: string | null;
  label: string | null;
}

export type RelationBasis = 'SAME_REQUEST' | 'CITES_SAME_DOCUMENT';

export interface RelatedFinding {
  id: string;
  kind: FindingKind;
  headline: string;
  severity: FindingSeverity;
  confidence: FindingConfidence;
  basis: RelationBasis;
  discoveredAt: string;
}

export interface DossierFalsification {
  statement: string;
  recordsToSeek: string[];
}

export type DossierGap =
  | { gap: 'unreconciledClaim'; portalKey: string; documentId: number; claimText: string; reason: string; detail: string }
  | { gap: 'notComputed'; portalKey: string; documentId: number; reason: string };

/** Observations assembled around one finding; never a legal claim. */
export interface LeadDossier {
  finding: Finding;
  evidence: EvidenceLink[];
  evidenceTruncated: boolean;
  related: RelatedFinding[];
  relatedTruncated: boolean;
  falsification: DossierFalsification;
  gaps: DossierGap[];
  gapsTruncated: boolean;
  assembledAt: string;
}
