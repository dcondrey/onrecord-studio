import type { PortalRegistration, SourceKind } from './investigation';

export interface CrawlPolicy {
  maximumPages: number;
  maximumDepth: number;
  maximumResources: number;
  followSupportedExternalPlatforms: boolean;
}

export interface SourceGuidance {
  // FEATURE(source): Add a guided selector preview type that returns matched
  // elements before a repair recipe is persisted.
  additionalSeedUrls: string[];
  allowedHostnames: string[];
  includeUrlPrefixes: string[];
  excludeUrlPrefixes: string[];
  linkSelector: string | null;
  downloadSelector: string | null;
}

export interface DiscoveredSourceLink {
  url: string;
  label: string;
  downloadCandidate: boolean;
  followCandidate: boolean;
}

export interface SourceDiscoveryPreview {
  source: PortalRegistration;
  detectedKind: SourceKind;
  confidenceBasisPoints: number;
  candidateLinks: DiscoveredSourceLink[];
  warnings: string[];
  repairAvailable: boolean;
}

export interface SourceIngestionStart {
  source: PortalRegistration;
  syncRunId: string;
  detectedKind: SourceKind;
  usedSavedGuidance: boolean;
}

export interface PastedContentInput {
  title: string;
  content: string;
  declaredMediaType?: string | null;
}
