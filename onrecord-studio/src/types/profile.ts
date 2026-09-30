import type { EntityValidationGrade } from './corpus';

export type AffiliationBasis = 'SELF_DECLARED' | 'EMAIL_DOMAIN_VERIFIED';

export interface Affiliation {
  id: string;
  name: string;
  aliases: string[];
  basis: AffiliationBasis;
  createdAt: string;
  /** Set exactly when basis is EMAIL_DOMAIN_VERIFIED, and only by the user's explicit link. */
  verifiedDomain: string | null;
}

/** A domain a mailbox code was confirmed for. The address and code are never stored. */
export interface VerifiedDomain {
  domain: string;
  verifiedAt: string;
  isFreeMail: boolean;
}

export type AffiliationMatchKind = 'PORTAL' | 'AGENCY' | 'ORGANIZATION';
export type MatchDecisionKind = 'CONFIRMED' | 'REJECTED';

export interface MatchDecision {
  affiliationId: string;
  kind: AffiliationMatchKind;
  normalizedName: string;
  decision: MatchDecisionKind;
  decidedAt: string;
}

export interface UserProfile {
  schemaVersion: number;
  displayName: string | null;
  affiliations: Affiliation[];
  matchDecisions: MatchDecision[];
  verifiedDomains: VerifiedDomain[];
  updatedAt: string;
}

export type ProfileStatus = 'UNSET' | 'SET';

export interface ProfileState {
  status: ProfileStatus;
  profile: UserProfile | null;
}

export interface AffiliationInput {
  id: string | null;
  name: string;
  aliases: string[];
}

export interface ProfileInput {
  displayName: string | null;
  affiliations: AffiliationInput[];
}

export type ProfileDeletionOutcome = 'REMOVED' | 'NOTHING_TO_REMOVE';

export type AffiliationMatchMethod = 'EXACT' | 'ALIAS' | 'FUZZY' | 'PREVIOUSLY_DECIDED';
export type CandidateDecision = 'PENDING' | 'CONFIRMED' | 'REJECTED';

export interface AffiliationMatchCandidate {
  affiliationId: string;
  kind: AffiliationMatchKind;
  displayValue: string;
  normalizedName: string;
  method: AffiliationMatchMethod;
  matchedTerm: string;
  validationGrade: EntityValidationGrade | null;
  documentCount: number | null;
  portalKey: string | null;
  agencyId: number | null;
  entityId: string | null;
  decision: CandidateDecision;
  currentlyPresent: boolean;
}

export type AffiliationMatchStatus =
  | 'UNSET'
  | 'NO_AFFILIATIONS_DECLARED'
  | 'NO_MATCH_YET'
  | 'HAS_CANDIDATES';

export interface AffiliationMatchList {
  status: AffiliationMatchStatus;
  candidates: AffiliationMatchCandidate[];
  searchedPortalCount: number;
  searchedAgencyCount: number;
  searchedEntityCount: number;
  limitations: string[];
}

export interface AffiliationMatchDecisionInput {
  affiliationId: string;
  kind: AffiliationMatchKind;
  normalizedName: string;
}

export type DisclosureStatus = 'UNSET' | 'DECLARED';

export interface DisclosedAffiliation {
  name: string;
  basis: AffiliationBasis;
}

export interface OwnAgency {
  portalKey: string;
  agencyId: number;
}

export interface AffiliationDisclosure {
  status: DisclosureStatus;
  sentence: string | null;
  affiliations: DisclosedAffiliation[];
  ownPortalKeys: string[];
  ownAgencies: OwnAgency[];
  ownEntityIds: string[];
}

export const PROFILE_LIMITS = {
  displayName: 120,
  affiliations: 8,
  affiliationName: 150,
  aliasesPerAffiliation: 8,
  alias: 80,
} as const;

export type WorkEmailCodeStatus =
  | 'SENT'
  | 'INVALID_EMAIL'
  | 'RATE_LIMITED'
  | 'NOT_SIGNED_IN'
  | 'SEND_FAILED';

export interface WorkEmailCodeReport {
  status: WorkEmailCodeStatus;
  expiresInSeconds: number | null;
  retryAfterSeconds: number | null;
}

export type WorkEmailConfirmStatus =
  | 'VERIFIED'
  | 'INVALID_INPUT'
  | 'NOT_SIGNED_IN'
  | 'NO_PENDING_REQUEST'
  | 'CODE_INVALID'
  | 'EXPIRED_OR_EXHAUSTED'
  | 'RATE_LIMITED';

export interface WorkEmailConfirmReport {
  status: WorkEmailConfirmStatus;
  domain: string | null;
  isFreeMail: boolean;
  attemptsRemaining: number | null;
  verifiedAt: string | null;
  retryAfterSeconds: number | null;
}

export interface AffiliationDomainLinkInput {
  affiliationId: string;
  domain: string;
}

export type AffiliationLinkStatus =
  | 'LINKED'
  | 'NOT_MATCHING'
  | 'FREE_MAIL_DOMAIN'
  | 'DOMAIN_NOT_VERIFIED'
  | 'AFFILIATION_NOT_FOUND';

export interface AffiliationLinkReport {
  status: AffiliationLinkStatus;
  message: string;
  profile: ProfileState;
}
