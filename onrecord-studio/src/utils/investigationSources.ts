import type { SanDiegoSource } from "./sanDiego";
import { regionalSource } from "./regionalSources";

const city = "https://www.sandiego.gov";
const bounded = {
  maximumPages: 1,
  maximumDepth: 0,
  maximumResources: 80,
  followSupportedExternalPlatforms: false,
};

// Publisher pages checked September 26, 2026. Coverage describes the route, not a completed import.
export const INVESTIGATION_SOURCES: readonly SanDiegoSource[] = [
  regionalSource(
    "sd-operating-actuals",
    "City operating actuals dataset",
    "https://data.sandiego.gov/datasets/operating-actuals/",
    "Housing budgets and outcomes",
    "City Department of Finance",
    "Reported operating expenditures and revenues for comparison with allocations.",
    "Match fiscal-year codes, expense accounts, funds, and accounting scope. Check whether the latest year is complete.",
    {
      evidenceType: "Dataset",
      crawlPolicy: { ...bounded, maximumResources: 20 },
    },
  ),
  regionalSource(
    "sd-get-it-done",
    "Get It Done service reports",
    "https://data.sandiego.gov/datasets/get-it-done-reports/",
    "Homelessness and public services",
    "City of San Diego",
    "Public non-emergency service reports, including available encampment and restroom categories.",
    "Reports measure requests for service, not unique encampments, people, completed abatements, or verified restroom availability. Check category definitions and duplicate reports.",
    {
      evidenceType: "Dataset",
      crawlPolicy: { ...bounded, maximumResources: 30 },
    },
  ),
  regionalSource(
    "sd-abatement-notices",
    "Encampment abatement and property notices",
    `${city}/homelessness-strategies-and-solutions/services/abatement/notices`,
    "Homelessness and public services",
    "City of San Diego",
    "Published notice templates for encampment abatement and impounded property.",
    "Templates explain the published process. Request event logs, notices actually issued, property disposition, costs, and referrals to evaluate practice.",
    {
      evidenceType: "Policy",
      crawlPolicy: bounded,
      coverage:
        "This page and directly linked documents. Individual abatement events and their outcomes are not supplied by this collection.",
    },
  ),
  regionalSource(
    "sd-restroom-availability",
    "Beach and bay restroom availability",
    `${city}/park-and-recreation/parks/regional/missionbay`,
    "Homelessness and public services",
    "City Parks and Recreation",
    "Official park information linking to the beach and bay restroom availability map.",
    "This covers beach and bay facilities, not all San Diego restrooms. Open the linked map; request facility hours, closure histories, and operating costs for a citywide analysis.",
    {
      evidenceType: "Reference website",
      access: "export",
      crawlPolicy: bounded,
      coverage:
        "The interactive map needs a browser. Import any available export; the park page alone is not a facility inventory.",
    },
  ),
  regionalSource(
    "sd-auditor-reports",
    "City Auditor reports",
    `${city}/auditor/reports`,
    "Elections and finance",
    "City Auditor",
    "Published audits, recommendations, and responses on City operations and spending.",
    "Retain audited period, finding, agency response, and recommendation status. A budget variance alone does not establish waste.",
    {
      evidenceType: "Audit collection",
      crawlPolicy: { ...bounded, maximumResources: 150 },
    },
  ),
];
