import type { PortalRegistration, SourceKind } from './investigation';

export interface CrawlPolicy {
  maximumPages: number;
  maximumDepth: number;
  maximumResources: number;
  followSupportedExternalPlatforms: boolean;
}

/**
 * Which slice of a portal's metadata API a synchronization starts from. Only the NextRequest API
 * path reads it; a crawled website or catalog has no page window. Mirrors
 * `MetadataPageWindow::default()` in src-tauri/src/types/source.rs.
 */
export interface MetadataPageWindow {
  startPage: number;
  pageSize: number;
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
  /**
   * What the operator is collecting for. A NextRequest portal is searched for it; a crawl
   * downloads only files whose link, address or page title names it. Absent collects everything.
   */
  subject?: string | null;
}

export interface CollectionPlanSource {
  portalKey: string;
  displayName: string;
  canonicalUrl: string;
  sourceKind: SourceKind;
  /** The last session queued work here that never ran. */
  hasUnfinishedWork: boolean;
  /** The subject this source was last collected for; `null` collected everything in scope. */
  subject: string | null;
}

export interface CollectionPlan {
  sources: CollectionPlanSource[];
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

/**
 * What one re-fetch of a collected document observed. DIFFERENT_BYTES is a digest comparison, not
 * a finding of alteration; UNREACHABLE says nothing about the record.
 */
export type SourceCheckOutcome = 'SAME_BYTES' | 'DIFFERENT_BYTES' | 'GONE' | 'UNREACHABLE';

export interface SourceCheck {
  checkId: number;
  runId: string;
  portalKey: string;
  documentId: number;
  requestedUrl: string;
  checkedAt: string;
  outcome: SourceCheckOutcome;
  httpStatus: number | null;
  collectedSha256: string;
  observedSha256: string | null;
  detail: string | null;
}

export interface SourceRecheckSummary {
  runId: string;
  portalKey: string;
  sameBytes: number;
  differentBytes: number;
  gone: number;
  unreachable: number;
  notChecked: number;
  stoppedEarly: string | null;
}
