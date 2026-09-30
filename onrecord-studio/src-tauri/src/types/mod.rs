//! Root type orchestration boundary exposing modular data engine shapes.

pub mod analysis;
pub mod analysis_workbench;
pub mod audit;
pub mod c2pa;
pub mod corpus;
pub mod intent;
pub mod investigation;
pub mod legal;
pub mod license;
pub mod maintenance;
pub mod nextrequest;
pub mod portal;
pub mod reconstruction;
pub mod research;
pub use reconstruction::*;
pub mod semantic;
pub mod source;

// Re-export core structures for convenient root usage
pub use analysis::{AgencyPerformanceProfile, AnalysisConfig};
pub use analysis_workbench::{
    AnalyticalColumnSummary, AnalyticalDatasetKind, AnalyticalDatasetSummary,
    AnalyticalImportCandidate, AnalyticalImportRequest, AnalyticalImportResult, AnalyticalSchema,
    SqlExportResult, SqlQueryResult, SqlTemplate,
};
pub use audit::{StateTransitionLog, TimelineAnomalyMarker, VerificationState};
pub use c2pa::{
    C2pAssertion, ProvenanceConfidence, ProvenanceManifest, RecordedDigestSource,
    SignatureVerificationState,
};
pub use corpus::{
    CorpusEntityEdge, CorpusEntityKind, CorpusEntityNeighborhood, CorpusEntityNode,
    CorpusEvidenceReference, CorpusIndexStatus, CorpusPattern, CorpusPatternKind,
    CorpusPatternQuery, EntityBroker, EntityPath, EntityPathStep, EntityResolutionBasis,
    EntityValidationGrade, RelationshipBasis, RelationshipKind,
};
pub use investigation::{
    AuxiliaryEventInput, AuxiliaryImportResult, AuxiliaryStreamKind, CaseEvidencePackageResult,
    CrossAgencyCorrelation, DocumentProvenanceVerification, DocumentTextChunk,
    LegalBriefExportResult, PackageVerificationResult, ProvenanceClaimGraph, ProvenanceClaimNode,
    ProvenanceIngredientEdge, ProvenanceVerificationRecord, RequestDocumentSummary,
    RequestSearchResult, RevokedSigningKey, SigningKeyRevocations, SigningKeyRotationResult,
    StatutoryExtensionReason, StatutoryExtensionRecord,
};
pub use legal::{
    LegalAuthorityImport, LegalAuthorityImportResult, LegalAuthorityKind, LegalAuthoritySummary,
    LegalAuthorityVersionDetail, LegalAuthorityVersionSummary, LegalComplianceFinding,
    LegalEvaluationResult, LegalFindingReviewStatus, LegalMandateInput, LegalMandateSeverity,
};
pub use license::{
    AccountStatus, CheckoutSession, DeleteAccountReport, LicensePlan, LicenseState, LicenseStatus,
    SignInOutcome, SignInReport,
};
pub use maintenance::{DataMaintenanceEvent, DataMaintenancePolicy, DataMaintenanceStatus};
pub use nextrequest::{
    DocumentMetadata, EmailMetadata, EmailParticipantMetadata, NextRequestResponse, PaginationMeta,
    PublicRequest,
};
pub use portal::{
    DocumentPurge, FrontierExclusionCount, PortalApiLayout, PortalRegistration,
    PortalRuntimeSnapshot, PortalSyncStart, RateLimitPolicy, SavedSource, SourceKind,
    SourceLifecycleState, SourcePurge, SourcePurgePreview, SourceRemoval, StoppedHost,
};
pub use research::*;
pub use semantic::{
    EmbeddingIndexReport, EmbeddingIndexStatus, EmbeddingModelStatus, HybridSearchFilter,
    HybridSearchResult,
};
pub use source::{
    CrawlPolicy, DiscoveredSourceLink, MetadataPageWindow, PastedContentInput,
    SourceDiscoveryPreview, SourceGuidance, SourceIngestionStart,
};

pub mod review;

pub mod financial;
pub use financial::*;

pub mod facilities;
pub use facilities::*;

pub mod casework;
pub use casework::*;

pub mod monitoring;
pub use monitoring::*;

pub mod profile;
pub mod report;
pub mod tracker;
pub use report::*;
