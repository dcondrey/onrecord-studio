import type { PortalRegistration, SourceKind } from './investigation';

export interface CrawlPolicy {
  maximumPages: number;
  maximumDepth: number;
  maximumResources: number;
  followSupportedExternalPlatforms: boolean;
}

export interface SourceGuidance {
  reuseForPublisherPattern?: boolean;
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
  /**
   * Refuse a discovered download whose bytes live on a host other than the source root's, unless
   * `allowedHostnames` names that host. Absent or false is the permissive default: government
   * portals routinely host their PDFs on S3, Azure Blob, Granicus or CivicPlus. It never changes
   * which links are followed, only which payloads are fetched.
   */
  restrictDownloadsToSourceHost?: boolean;
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
