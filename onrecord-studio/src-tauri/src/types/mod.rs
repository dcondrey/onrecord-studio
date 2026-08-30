//! Root type orchestration boundary exposing modular data engine shapes.

pub mod analysis;
pub mod audit;
pub mod c2pa;
pub mod corpus;
pub mod investigation;
pub mod legal;
pub mod license;
pub mod nextrequest;
pub mod portal;
pub mod source;

// Re-export core structures for convenient root usage
pub use analysis::{AgencyPerformanceProfile, AnalysisConfig, OutlierCluster};
pub use audit::{CaseAuditRecord, StateTransitionLog, TimelineAnomalyMarker, VerificationState};
pub use c2pa::{
    C2pAssertion, ProvenanceConfidence, ProvenanceManifest, RecordedDigestSource,
    SignatureVerificationState,
};
pub use corpus::{
    CorpusEntityKind, CorpusEvidenceReference, CorpusIndexStatus, CorpusPattern, CorpusPatternKind,
    CorpusPatternQuery, EntityResolutionBasis, EntityValidationGrade, RelationshipBasis,
};
pub use investigation::{
    AuxiliaryEventInput, AuxiliaryImportResult, AuxiliaryStreamKind, CrossAgencyCorrelation,
    DocumentProvenanceVerification, DocumentTextChunk, LegalBriefExportResult,
    ProvenanceClaimGraph, ProvenanceClaimNode, ProvenanceIngredientEdge,
    ProvenanceVerificationRecord, RequestDocumentSummary, RequestSearchResult,
    StatutoryExtensionReason, StatutoryExtensionRecord,
};
pub use legal::{
    LegalAuthorityImport, LegalAuthorityImportResult, LegalAuthorityKind, LegalAuthoritySummary,
    LegalAuthorityVersionDetail, LegalAuthorityVersionSummary, LegalComplianceFinding,
    LegalEvaluationResult, LegalFindingReviewStatus, LegalMandateInput, LegalMandateSeverity,
};
pub use license::{LicenseState, LicenseStatus};
pub use nextrequest::{
    DocumentMetadata, EmailMetadata, EmailParticipantMetadata, NextRequestResponse, PaginationMeta,
    PublicRequest,
};
pub use portal::{
    DocumentPurge, FrontierExclusionCount, PortalApiLayout, PortalRegistration,
    PortalRuntimeSnapshot, PortalSyncStart, RateLimitPolicy, SavedSource, SourceKind,
    SourceLifecycleState, SourcePurge, SourcePurgePreview, SourceRemoval, StoppedHost,
};
pub use source::{
    CrawlPolicy, DiscoveredSourceLink, MetadataPageWindow, PastedContentInput,
    SourceDiscoveryPreview, SourceGuidance, SourceIngestionStart,
};
