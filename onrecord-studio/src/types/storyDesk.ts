import type { FindingConfidence, FindingKind, FindingSeverity, FindingSubject } from './findings';

/** One observable reason a lead ranks where it does, with the fact behind it. */
export interface InterestFactor {
  name: string;
  basisPoints: number;
  evidence: string;
}

/**
 * A ranking heuristic over facts the detectors observed. It orders leads for a reader's
 * attention; it is not a measurement of newsworthiness and never a claim about the record.
 */
export interface PublicInterestScore {
  totalBasisPoints: number;
  factors: InterestFactor[];
}

export interface StoryLead {
  findingId: string;
  kind: FindingKind;
  headline: string;
  severity: FindingSeverity;
  confidence: FindingConfidence;
  subject: FindingSubject;
  portalKey: string | null;
  publicInterest: PublicInterestScore;
  relatedFindingIds: string[];
  firstSeenAt: string;
  lastSeenAt: string;
}
