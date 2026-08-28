// TODO(telemetry): Generate this discriminated event union from the native
// protocol schema and fail the build when a Rust event is not handled in App.
export interface PortalApiLayout {
  requestsEndpoint: string;
  requestsPointer: string;
  metadataPointer: string | null;
  pageParameter: string;
  pageSizeParameter: string;
}

export interface PortalRegistration {
  portalKey: string;
  displayName: string;
  canonicalUrl: string;
  originUrl: string;
  apiLayout: PortalApiLayout;
  verifiedAt: string;
  sourceKind: SourceKind;
}

/// Whether a saved source is still being synchronized, and if not, why not.
///
/// Durable on the native side: a source halted only in memory was resumed by the next launch,
/// which found its frontier and read it as an interrupted crawl.
export type SourceLifecycleState = 'ACTIVE' | 'PAUSED' | 'REMOVED';

/// A registration paired with its lifecycle state. Kept side by side rather than merged because
/// the registration is compared for equality to decide whether a source key still names the same
/// verified endpoint, and mutable state has no business in that comparison.
export interface SavedSource {
  registration: PortalRegistration;
  lifecycleState: SourceLifecycleState;
}

/// What withdrawing a source discarded, and what it kept. The retained document count is carried
/// so the interface can say that removing a source does not remove its evidence.
export interface SourceRemoval {
  sourceKey: string;
  discardedFrontierTasks: number;
  retainedDocuments: number;
}

export type SourceKind =
  | 'NEXT_REQUEST'
  | 'WEB_CRAWL'
  | 'DIRECT_FILE'
  | 'OPEN_DATA_CATALOG'
  | 'ARC_GIS'
  | 'TABLEAU'
  | 'GOOGLE_DRIVE'
  | 'LOCAL_FILE'
  | 'PASTED_CONTENT';

export interface RateLimitPolicy {
  burstCapacity: number;
  refillTokensPerSecond: number;
}

export interface PortalRuntimeSnapshot {
  portal: PortalRegistration;
  syncRunId: string;
  queuedTasks: number;
  activeTasks: number;
  completedTasks: number;
  failedTasks: number;
  pagesProcessed: number;
  requestsProcessed: number;
  documentsProcessed: number;
  isRunning: boolean;
  rateLimitTokens: number;
  /** How many discovered links each frontier rule refused, so a filtered crawl reports what it
   * declined rather than presenting the survivors as the whole of what was there. */
  frontierExclusions: FrontierExclusionCount[];
  /** Hosts this run stopped contacting, and why. A stopped host bounds coverage. */
  stoppedHosts: StoppedHost[];
}

export interface FrontierExclusionCount {
  rule: string;
  excludedLinks: number;
}

export interface StoppedHost {
  host: string;
  requestsSpent: number;
  reason: string;
}

type IngestionTelemetryEvent =
  | { type: 'portalVerified'; payload: { portal: PortalRegistration } }
  | { type: 'syncStarted'; payload: { portalKey: string; syncRunId: string } }
  | { type: 'pageStarted'; payload: { portalKey: string; page: number } }
  | { type: 'pageCompleted'; payload: { portalKey: string; page: number; recordsFound: number } }
  | { type: 'documentStarted'; payload: { portalKey: string; documentId: number; fileName: string } }
  | { type: 'documentExtracted'; payload: { portalKey: string; documentId: number; characterCount: number; wasOcrProcessed: boolean } }
  | { type: 'documentExpanded'; payload: { portalKey: string; documentId: number; format: string; entryCount: number } }
  | { type: 'documentDeferred'; payload: { portalKey: string; documentId: number; format: string; reason: string } }
  | { type: 'documentSalvaged'; payload: { portalKey: string; documentId: number; format: string; primaryError: string } }
  | { type: 'documentCompleted'; payload: { portalKey: string; documentId: number; bytesSaved: number } }
  | { type: 'syncStopped'; payload: { portalKey: string; syncRunId: string } }
  | {
      type: 'hostStopped';
      payload: { portalKey: string; host: string; requestsSpent: number; reason: string };
    }
  | { type: 'taskFailed'; payload: { portalKey: string; targetId: string; errorMsg: string } }
  | { type: 'taskRetryScheduled'; payload: { portalKey: string; targetId: string; attempt: number; delaySeconds: number; errorMsg: string } }
  | { type: 'sourceNeedsGuidance'; payload: { portalKey: string; sourceUrl: string; reason: string } }
  | { type: 'legacyCoreDatabaseMigrated'; payload: { backupPath: string; copiedTables: number; copiedRows: number; tablesAbsentFromLegacy: string[]; tablesDroppedFromLegacy: string[] } }
  | { type: 'corpusReconciliationStarted'; payload: Record<string, never> }
  | { type: 'corpusReconciliationCompleted'; payload: { indexedDocuments: number; skippedDocuments: number; failedDocuments: number; firstError: string | null } };

export type IngestionTelemetry = {
  // NOTE(telemetry): `sequence` is a string because the bridge transports it
  // uniformly across runtimes; keep numeric ordering in the consumer only.
  protocolVersion: 1;
  sequence: string;
  emittedAt: string;
} & IngestionTelemetryEvent;

export interface RequestSearchResult {
  requestId: number;
  portalKey: string;
  agencyId: number | null;
  agencyName: string;
  title: string;
  descriptionPreview: string;
  status: string;
  createdAt: string;
  closedAt: string | null;
  documentCount: number;
  integrityFlags: string[];
}

/**
 * How much a provenance result is worth as evidence, ordered weakest to strongest.
 *
 * Mirrors the Rust `ProvenanceConfidence` ladder. Read this before any boolean on
 * `DocumentProvenanceVerification`: the booleans mix checks this application performed with claims
 * it merely read out of local storage, and only the ladder distinguishes them.
 */
export type ProvenanceConfidence =
  | 'EVIDENCE_FILE_UNAVAILABLE'
  | 'DIGEST_MISMATCH'
  | 'NO_RECORDED_DIGEST'
  | 'RECORDED_CLAIM_ONLY'
  | 'LOCAL_DIGEST_MATCH'
  | 'VERIFIED_SIGNATURE'
  /** Requires a configured trust anchor. This build supplies none, so it never reports this. */
  | 'TRUSTED_CHAIN_OF_CUSTODY';

/** Which recorded digest the stored bytes were compared against. */
export type RecordedDigestSource = 'NONE' | 'PROVENANCE_MANIFEST' | 'INGESTION_RECORD';

/** Whether a manifest signature was actually checked, and what the check said. */
export type SignatureVerificationState = 'NOT_ATTEMPTED' | 'VALID' | 'INVALID';

export interface DocumentProvenanceVerification {
  documentId: number;
  portalKey: string;
  expectedSha256: string | null;
  /** Empty when no readable evidence file existed; read `evidenceFilePresent` to tell them apart. */
  computedSha256: string;
  /** False means nothing was hashed, so `hashMatches` is false because no check ran. */
  evidenceFilePresent: boolean;
  hashMatches: boolean;
  recordedDigestSource: RecordedDigestSource;
  /** A manifest row exists locally. A stored claim, not a check. */
  manifestPresent: boolean;
  /** The manifest *claims* a signature. See `signatureVerification` for whether one was checked. */
  isCryptographicallySigned: boolean;
  signatureVerification: SignatureVerificationState;
  signatureIssuer: string | null;
  signingTimestamp: string | null;
  assertionCount: number;
  invalidAssertionCount: number;
  /** Digest recorded for the normalized text projection that search and briefs quote. */
  textProjectionSha256: string | null;
  /** `null` means the comparison could not be made, not that it passed. */
  textProjectionMatches: boolean | null;
  provenanceConfidence: ProvenanceConfidence;
  /**
   * Independent third-party evidence of custody. True only at `TRUSTED_CHAIN_OF_CUSTODY`, which
   * requires configured trust anchors. This build supplies none and offers nowhere to configure
   * them, so it is false for every document. Present the ladder instead of this flag.
   */
  chainOfCustodyConfirmed: boolean;
  verificationErrors: string[];
}

/**
 * One row of the append-only provenance verification log.
 *
 * Deliberately not a `DocumentProvenanceVerification`. Rows written before classification existed
 * carry no confidence level; those fields are `null` and `predatesClassification` is true, which is
 * not the same as a recorded low-confidence result.
 */
export interface ProvenanceVerificationRecord {
  verificationId: string;
  documentId: number;
  portalKey: string;
  verifiedAt: string;
  expectedSha256: string | null;
  computedSha256: string;
  evidenceFilePresent: boolean | null;
  hashMatches: boolean;
  manifestPresent: boolean;
  isCryptographicallySigned: boolean;
  signatureIssuer: string | null;
  signingTimestamp: string | null;
  assertionCount: number | null;
  invalidAssertionCount: number;
  recordedDigestSource: RecordedDigestSource | null;
  signatureVerification: SignatureVerificationState | null;
  provenanceConfidence: ProvenanceConfidence | null;
  textProjectionSha256: string | null;
  textProjectionMatches: boolean | null;
  chainOfCustodyConfirmed: boolean;
  verificationErrors: string[];
  predatesClassification: boolean;
}

export interface LegalBriefExportResult {
  requestId: number;
  portalKey: string;
  outputPath: string;
  reportSha256: string;
  generatedAt: string;
  includedViolationMarkers: string[];
}

export interface RequestDocumentSummary {
  documentId: number;
  portalKey: string;
  fileName: string;
  fileSize: number | null;
  contentType: string | null;
  visibility: string;
  downloadStatus: string;
  rawFileSha256: string | null;
  extractedTextSha256: string | null;
}

export interface DocumentTextChunk {
  documentId: number;
  portalKey: string;
  characterOffset: number;
  returnedCharacterCount: number;
  nextCharacterOffset: number | null;
  totalCharacterCount: number;
  maskedText: string;
  maskedFindingCount: number;
}

export interface RecoveredTextSegment {
  revisionNumber: number;
  snapshotEndOffset: number;
  recoveredCharacterCount: number;
  recoveredTextSha256: string;
  maskedFindingCount: number;
  maskedText: string;
}

export interface RecoveredMetadataArtifact {
  byteOffset: number;
  category: string;
  valueSha256: string;
  maskedValue: string;
  maskedFindingCount: number;
}

export interface RecoveredVersionProfile {
  sourceSha256: string;
  sourceSizeBytes: number;
  totalEofMarkers: number;
  historicalVersionsFound: number;
  inspectedHistoricalVersions: number;
  recoveredTextSegments: RecoveredTextSegment[];
  postEofMetadataArtifacts: RecoveredMetadataArtifact[];
  analysisTruncated: boolean;
  warnings: string[];
}

export interface RedactionIntegrityReport {
  analysisVersion: string;
  sourceSha256: string;
  scannedPageCount: number;
  coverageComplete: boolean;
  requiresManualReview: boolean;
  isLeakyRedaction: boolean;
  extractedHiddenText: string;
  boundingBoxClashesCount: number;
  metadataLeaks: string[];
  isolatedStructuralAnomalies: string[];
  pageFindings: RedactionPageFinding[];
  calculatedRedactionRatio: number;
}

export interface RedactionPageFinding {
  pageNumber: number;
  potentialIntersections: number;
  darkOverlayIntersections: number;
  redactionAnnotationIntersections: number;
  invisibleTextIntersections: number;
  unappliedRedactionAnnotations: number;
  formInvocationsAnalyzed: number;
  coverageComplete: boolean;
  intersections: RedactionIntersectionEvidence[];
}

export interface RedactionIntersectionEvidence {
  kind: string;
  textBounds: [number, number, number, number];
  maskBounds: [number, number, number, number];
}
