import type { PortalRegistration, SourceKind } from './investigation';

export interface CrawlPolicy {
  maximumPages: number;
  maximumDepth: number;
  maximumResources: number;
  followSupportedExternalPlatforms: boolean;
}

export interface SourceGuidance {
  additionalSeedUrls: string[];
  allowedHostnames: string[];
  includeUrlPrefixes: string[];
  excludeUrlPrefixes: string[];
  linkSelector: string | null;
  downloadSelector: string | null;
  includeFilenameGlobs: string[];
  excludeFilenameGlobs: string[];
  includeFilenamePatterns: string[];
  excludeFilenamePatterns: string[];
  includeContentKeywords: string[];
  excludeContentKeywords: string[];
  includeContentPatterns: string[];
  excludeContentPatterns: string[];
  includeOrganizations: string[];
  excludeOrganizations: string[];
  minimumFileSizeBytes: number | null;
  maximumFileSizeBytes: number | null;
  documentDateFrom: string | null;
  documentDateTo: string | null;
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
  selectorPreviewApplied: boolean;
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
