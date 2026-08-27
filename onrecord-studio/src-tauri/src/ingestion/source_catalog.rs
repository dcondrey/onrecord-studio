//! Placeholder registry for public-record and civic-data connector profiles.
//!
//! Profiles describe acquisition requirements; they are not claims that a live connector exists.
//! Every profile must still pass host validation, terms/licensing review, rate limits, and any
//! credential/PII policy before it is enabled.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorFamily {
    GenericCrawl,
    PublicApi,
    OpenDataCatalog,
    ArcGis,
    Tableau,
    ApiKeyOrAccount,
    AuthenticatedApi,
    PaidOrCredentialed,
    RestrictedOrPartnerOnly,
    ManualImport,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccessRequirement {
    Public,
    PublicWithRateLimits,
    ApiKeyOrAccount,
    PaidAccount,
    AgencyOrPartnerApproval,
    LocalFileExport,
}

#[derive(Debug, Clone, Copy)]
pub struct SourceProfile {
    pub slug: &'static str,
    pub display_name: &'static str,
    pub family: ConnectorFamily,
    pub access: AccessRequirement,
    pub notes: &'static str,
}

/// Returns planned connector profiles for UI discovery and future adapter dispatch.
pub const fn planned_source_profiles() -> &'static [SourceProfile] {
    // FEATURE(source-catalog): Replace this static list with versioned connector manifests that
    // include endpoint templates, pagination, retry policy, schemas, attribution, and retention.
    // NOTE(source-catalog): “Public” means reachable without login, not unrestricted scraping;
    // robots, terms, API fair-use rules, copyright, and court-access rules still apply.
    const PROFILES: &[SourceProfile] = &[
        SourceProfile {
            slug: "muckrock",
            display_name: "MuckRock",
            family: ConnectorFamily::AuthenticatedApi,
            access: AccessRequirement::ApiKeyOrAccount,
            notes: "Public request pages plus optional account/API access.",
        },
        SourceProfile {
            slug: "documentcloud",
            display_name: "DocumentCloud",
            family: ConnectorFamily::AuthenticatedApi,
            access: AccessRequirement::ApiKeyOrAccount,
            notes: "Public documents and API; preserve document/page metadata.",
        },
        SourceProfile {
            slug: "government-attic",
            display_name: "Government Attic",
            family: ConnectorFamily::GenericCrawl,
            access: AccessRequirement::Public,
            notes: "Static archive pages and downloadable records.",
        },
        SourceProfile {
            slug: "agency-reading-room",
            display_name: "Agency FOIA reading rooms",
            family: ConnectorFamily::GenericCrawl,
            access: AccessRequirement::Public,
            notes: "Agency-specific HTML, search, and download layouts.",
        },
        SourceProfile {
            slug: "data-gov",
            display_name: "Data.gov",
            family: ConnectorFamily::OpenDataCatalog,
            access: AccessRequirement::PublicWithRateLimits,
            notes: "Catalog metadata should dispatch to the publisher dataset.",
        },
        SourceProfile {
            slug: "usaspending",
            display_name: "USAspending.gov",
            family: ConnectorFamily::PublicApi,
            access: AccessRequirement::PublicWithRateLimits,
            notes: "Paginated API and bulk downloads.",
        },
        SourceProfile {
            slug: "federal-register",
            display_name: "Federal Register",
            family: ConnectorFamily::PublicApi,
            access: AccessRequirement::PublicWithRateLimits,
            notes: "Documents, docket metadata, and API pagination.",
        },
        SourceProfile {
            slug: "opensecrets",
            display_name: "OpenSecrets",
            family: ConnectorFamily::ApiKeyOrAccount,
            access: AccessRequirement::ApiKeyOrAccount,
            notes: "Provider terms and API quotas must be enforced.",
        },
        SourceProfile {
            slug: "sec-edgar",
            display_name: "SEC EDGAR",
            family: ConnectorFamily::PublicApi,
            access: AccessRequirement::PublicWithRateLimits,
            notes: "SEC fair-access User-Agent and rate policy required.",
        },
        SourceProfile {
            slug: "black-vault",
            display_name: "The Black Vault",
            family: ConnectorFamily::GenericCrawl,
            access: AccessRequirement::Public,
            notes: "Static FOIA pages and document downloads.",
        },
        SourceProfile {
            slug: "nara",
            display_name: "National Archives databases",
            family: ConnectorFamily::PublicApi,
            access: AccessRequirement::PublicWithRateLimits,
            notes: "Archive-specific APIs and digital-object links.",
        },
        SourceProfile {
            slug: "fbi-vault",
            display_name: "FBI Vault",
            family: ConnectorFamily::GenericCrawl,
            access: AccessRequirement::Public,
            notes: "Reading-room index and document assets.",
        },
        SourceProfile {
            slug: "cia-reading-room",
            display_name: "CIA FOIA Reading Room",
            family: ConnectorFamily::GenericCrawl,
            access: AccessRequirement::Public,
            notes: "Reading-room search and downloadable records.",
        },
        SourceProfile {
            slug: "courtlistener-recap",
            display_name: "CourtListener / RECAP",
            family: ConnectorFamily::AuthenticatedApi,
            access: AccessRequirement::ApiKeyOrAccount,
            notes: "API/RECAP availability and document rights vary by item.",
        },
        SourceProfile {
            slug: "pacer",
            display_name: "PACER",
            family: ConnectorFamily::PaidOrCredentialed,
            access: AccessRequirement::PaidAccount,
            notes: "Credentialed, fee-bearing court system; no bypass automation.",
        },
        SourceProfile {
            slug: "govqa",
            display_name: "GovQA",
            family: ConnectorFamily::AuthenticatedApi,
            access: AccessRequirement::AgencyOrPartnerApproval,
            notes: "Tenant-specific portals and session-bound exports.",
        },
        SourceProfile {
            slug: "justfoia",
            display_name: "JustFOIA",
            family: ConnectorFamily::AuthenticatedApi,
            access: AccessRequirement::AgencyOrPartnerApproval,
            notes: "Tenant-specific portal, export, and access controls.",
        },
        SourceProfile {
            slug: "311-get-it-done",
            display_name: "311 / Get It Done",
            family: ConnectorFamily::ArcGis,
            access: AccessRequirement::PublicWithRateLimits,
            notes: "Dataset/API shape varies by city and publisher.",
        },
        SourceProfile {
            slug: "hud-exchange",
            display_name: "HUD Exchange",
            family: ConnectorFamily::GenericCrawl,
            access: AccessRequirement::Public,
            notes: "Reports, guidance, and linked datasets.",
        },
        SourceProfile {
            slug: "hmis",
            display_name: "HMIS data",
            family: ConnectorFamily::RestrictedOrPartnerOnly,
            access: AccessRequirement::AgencyOrPartnerApproval,
            notes: "Do not ingest participant-level data without explicit governance.",
        },
        SourceProfile {
            slug: "hud-spm-hic-pit",
            display_name: "HUD SPM/HIC/PIT datasets",
            family: ConnectorFamily::OpenDataCatalog,
            access: AccessRequirement::Public,
            notes: "Published aggregates and data dictionaries.",
        },
        SourceProfile {
            slug: "eviction-lab",
            display_name: "Eviction Lab",
            family: ConnectorFamily::OpenDataCatalog,
            access: AccessRequirement::Public,
            notes: "Dataset license and attribution must travel with imports.",
        },
        SourceProfile {
            slug: "national-equity-atlas",
            display_name: "National Equity Atlas",
            family: ConnectorFamily::GenericCrawl,
            access: AccessRequirement::Public,
            notes: "Reports, dashboards, and downloadable tables.",
        },
        SourceProfile {
            slug: "regional-open-data",
            display_name: "Regional and municipal open-data libraries",
            family: ConnectorFamily::OpenDataCatalog,
            access: AccessRequirement::PublicWithRateLimits,
            notes: "Prefer catalog/API endpoints over broad crawling; San Diego is one fixture, not a special case.",
        },
        SourceProfile {
            slug: "community-organizations",
            display_name: "Community, shelter, and outreach organizations",
            family: ConnectorFamily::ManualImport,
            access: AccessRequirement::LocalFileExport,
            notes: "Use consented exports for local partners; do not assume a San Diego-specific provider.",
        },
        SourceProfile {
            slug: "local-311-and-crm",
            display_name: "Local 311 and service-request systems",
            family: ConnectorFamily::ArcGis,
            access: AccessRequirement::PublicWithRateLimits,
            notes: "Support ArcGIS, Socrata, CKAN, and vendor-specific exports by profile.",
        },
    ];
    PROFILES
}
