import type { SanDiegoSource } from './sanDiego';
import { regionalSource } from './regionalSources';

const city = 'https://www.sandiego.gov';
const county = 'https://www.sandiegocounty.gov';
const topic = 'Housing budgets and outcomes';

// Case-sensitive filenames copied from the publishers' links. Retain the operator's original
// spelling as an exact alias; do not lowercase arbitrary incoming paths or guess missing files.
const correctedPublications: Record<string, string> = {
  'rtfh-2024-performance': 'https://www.rtfhsd.org/wp-content/uploads/2025/07/2024-Data-Performance-Report_v3.pdf',
  'rtfh-hmis-dec2024': 'https://www.rtfhsd.org/wp-content/uploads/2025/01/RTFH-HMIS-Data-Newsletter-Dec-2024.pdf',
  'rtfh-2024-pit-cities-revised': 'https://www.rtfhsd.org/wp-content/uploads/2024/09/2024-PITC-Regional-Cities-Breakdown-revised-Final-June-17-1.pdf',
  'sdhc-fy26-budget': 'https://sdhc.org/wp-content/uploads/2025/01/FY26-Budget-City-Format.pdf',
  'sdhc-fy24-budget': 'https://sdhc.org/wp-content/uploads/2023/05/SDHC-FY2024-Budget-Report.pdf',
  'sdhc-fy25-budget': 'https://sdhc.org/wp-content/uploads/2024/06/HAR24-005-SDHC-FY25-Budget-Report.pdf',
  'sdhc-homelessness-at-a-glance': 'https://sdhc.org/wp-content/uploads/2022/12/SDHC-at-a-glance-homelessness-solutions.pdf',
  'county-draft-housing-element': 'https://www.sandiegocounty.gov/content/dam/sdc/pds/GPUpdate2021/DraftHousingElement-StrikeoutVersion.pdf',
  'county-homeless-health-2024-26': 'https://www.sandiegocounty.gov/content/dam/sdc/hhsa/programs/phs/CHS/Persons%20Experiencing%20Homelessness%20in%20SDC%202024-2026.pdf',
};

const unavailablePublications = new Set([
  'county-fy25-29-consolidated-plan', 'escondido-meeting-housing-attachment',
]);

function publication(
  id: string, label: string, url: string, publisher: string,
  evidenceType: string, period: string, analysisNotes: string,
): SanDiegoSource {
  const pdf = new URL(url).pathname.toLowerCase().endsWith('.pdf');
  const unavailable = unavailablePublications.has(id);
  return regionalSource(id, label, correctedPublications[id] ?? url, topic, publisher,
    `${evidenceType}${period ? ` · ${period}` : ''}. Preserve the selected publication and its original source.`,
    analysisNotes, {
      evidenceType, period,
      aliases: [...new Set([url, url.replace('://www.', '://')])],
      access: unavailable ? 'export' : 'public',
      crawlPolicy: { maximumPages: 1, maximumDepth: 0, maximumResources: pdf ? 1 : 40, followSupportedExternalPlatforms: false },
      coverage: unavailable
        ? 'The supplied document URL returned 404 during verification. Import an available downloaded copy through Import files; retain its original URL and document identity.'
        : pdf
        ? 'This specific PDF, subject to publisher availability. Review extracted text and page references; PDF tables require verification before numerical analysis.'
        : 'This selected page and its directly linked documents. Collection limits and publisher availability apply.',
    });
}

const adopted = 'An adopted appropriation authorizes spending; it is not an expenditure. Keep fiscal year, fund, department, ongoing versus one-time funding, and transfers. Do not add summary totals to their component lines.';
const plan = 'A plan describes proposed activities and funding. Retain plan period, amendments, geography, program eligibility, and later performance or expenditure reports.';
const performance = 'Retain counting unit, geography, period, methodology, and revisions. PIT estimates, HMIS service activity, shelter capacity, and housing placements cannot be added as unique people.';

// Exact publications requested by the operator. Selection never expands into a publisher-wide crawl.
export const HOUSING_SOURCES: readonly SanDiegoSource[] = [
  publication('fy25-city-agencies', 'FY2025 City agencies budget', `${city}/sites/default/files/2024-08/fy25ab_v1cityagencies.pdf`, 'City of San Diego', 'Adopted budget', 'FY2025', adopted),
  publication('fy25-annual-action-plan', 'Annual action plan — August 2024 release', `${city}/sites/default/files/2024-08/annual-action-plan.pdf`, 'City of San Diego', 'Plan', 'August 2024 release', plan),
  publication('fy26-budget-guide', 'FY2026 guide to the budget', `${city}/sites/default/files/2025-08/fy26ab_v2guidetothebudget.pdf`, 'City of San Diego', 'Budget explanation', 'FY2026', 'Use the guide to interpret fund structure and budget tables; its explanations are not additional spending lines.'),
  publication('iba-legacy-budget-guide', 'IBA legacy budget guide', `${city}/sites/default/files/legacy/iba/pdf/budgetguide.pdf`, 'City Independent Budget Analyst', 'Budget explanation', 'Legacy edition', 'Confirm the edition and applicable fiscal year in the document. Historical explanations may use earlier departmental or fund structures.'),
  regionalSource('sd-operating-budget', 'City operating budget dataset', 'https://data.sandiego.gov/datasets/operating-budget', topic, 'City Department of Finance',
    'Published operating budget CSV, dictionary, and reference-table guidance.',
    'Rows are expense or revenue account allocations. Filter report_fy and budget_cycle before summing amount; keep fund_number, funds_center_number, and account_number as identifiers. Join reference tables on their documented keys.',
    { evidenceType: 'Dataset', period: 'Multiple fiscal years', crawlPolicy: { maximumPages: 2, maximumDepth: 1, maximumResources: 20, followSupportedExternalPlatforms: false }, guidance: { includeUrlPrefixes: ['https://data.sandiego.gov/datasets/operating-budget', 'https://data.sandiego.gov/resources/', 'https://seshat.datasd.org/'] } }),
  publication('fy26-city-budget-overview', 'FY2026 City budget overview', `${city}/sites/default/files/2025-08/fy26ab_v1cbo.pdf`, 'City of San Diego', 'Adopted budget', 'FY2026', adopted),
  publication('iba-2024-budget-guide', 'IBA budget guide — October 2024', `${city}/sites/default/files/2024-10/iba-budget-guide-eng.pdf`, 'City Independent Budget Analyst', 'Budget explanation', 'October 2024 edition', 'Use this edition’s definitions and fiscal-year context. Do not assume amounts or policies match later adopted budgets.'),
  publication('fy26-city-agencies', 'FY2026 City agencies budget', `${city}/sites/default/files/2025-08/fy26ab_v1cityagencies.pdf`, 'City of San Diego', 'Adopted budget', 'FY2026', adopted),
  publication('rtfh-2024-performance', 'RTFH 2024 data and performance report', 'https://rtfhsd.org/wp-content/uploads/2025/07/2024-data-performance-report_v3.pdf', 'Regional Task Force on Homelessness', 'Performance report', '2024 · version 3', performance),
  publication('county-fy25-29-consolidated-plan', 'County consolidated and annual plans', `${county}/content/dam/sdc/sdhcd/new-docs/cd/fy25-29_conplan_fy25-26_annualplan_final.pdf`, 'County of San Diego', 'Plan', 'FY2025–2029 / FY2025–2026', plan),
  publication('fy26-public-budget-guide', 'Public guide to the FY2026 adopted budget', `${city}/sites/default/files/2025-12/publics-guide-to-the-budget-process-and-the-fy-2026-adopted-budget.pdf`, 'City Independent Budget Analyst', 'Budget explanation', 'FY2026 · December 2025 edition', 'Keep this guide’s edition and page references when interpreting the adopted budget. It is a secondary explanation of budget records.'),
  publication('fy25-community-action-plan', 'FY2025 consolidated-plan annual action plan', `${city}/sites/default/files/2024-07/fy25cpaap_1.pdf`, 'City of San Diego', 'Plan', 'FY2025', plan),
  publication('fy21-caper-attachment', 'FY2021 CAPER attachment 3', `${city}/sites/default/files/cosdfy21caper_att3.pdf`, 'City of San Diego', 'Performance report', 'FY2021 · attachment 3', 'This is an attachment, not the complete CAPER. Check which measures are expenditures versus accomplishments and retain the parent report context.'),
  publication('county-homeless-health-2024-26', 'County health: persons experiencing homelessness', `${county}/content/dam/sdc/hhsa/programs/phs/chs/persons%20experiencing%20homelessness%20in%20sdc%202024-2026.pdf`, 'County HHSA', 'Health report', '2024–2026 edition', 'Retain each indicator’s actual observation period, population, denominator, and suppression. The edition range is not automatically the measurement period.'),
  publication('fy27-sheltering-announcement', 'FY2027 sheltering options announcement', `${city}/sites/default/files/2026-06/2026-6-16-city-of-san-diego-funds-2-700-sheltering-options-for-individuals-experiencing-homelessness-in-fy-2027-budget.pdf`, 'City of San Diego', 'Agency statement', 'FY2027 · June 16, 2026', 'Check the announced count against funded capacity and operating dates. Sheltering options, occupied beds, and unique people served are different measures.'),
  publication('fy26-hssd', 'FY2026 HSSD capital improvements', `${city}/sites/default/files/2025-08/fy26ab_v3hssd.pdf`, 'City of San Diego', 'Adopted capital budget', 'FY2026', `${adopted} This is the CIP chapter. Compare capital projects separately from the FY2025 HSSD operating chapter.`),
  publication('fy26-caper', 'FY2026 CAPER', `${city}/sites/default/files/2026-08/fy2026-caper.pdf`, 'City of San Diego', 'Performance report', 'FY2026', 'Confirm draft/final status and covered dates inside the report. Separate expenditures, grant balances, and reported accomplishments; none alone measures cost effectiveness.'),
  publication('city-home-arp', 'City HOME-ARP allocation plan', `${city}/sites/default/files/cosd_home_arp.pdf`, 'City of San Diego', 'Plan', 'HOME-ARP', 'Retain allocation, activity, eligible population, amendments, and obligation/spending dates. One-time grant awards are not recurring annual expenditures.'),
  publication('iba-fy24-homelessness', 'IBA FY2024 homelessness programs and funding', `${city}/sites/default/files/23-21_fy_2024_homelessness_programs_and_funding.pdf`, 'City Independent Budget Analyst', 'Funding analysis', 'FY2024 · report 23-21', 'Track each funding source and program; distinguish City, Housing Commission, and pass-through funding to avoid double counting.'),
  publication('fy26-capital-budget', 'FY2026 capital improvements budget volume', `${city}/sites/default/files/2025-08/fy26ab_v3.pdf`, 'City of San Diego', 'Adopted capital budget', 'FY2026', `${adopted} The HSSD CIP chapter is also listed separately; it is part of this volume. Capital and operating budgets have different scopes.`),
  publication('sdhc-fy26-budget', 'SDHC FY2026 proposed budget — City format', 'https://sdhc.org/wp-content/uploads/2025/01/fy26-budget-city-format.pdf', 'San Diego Housing Commission', 'Budget proposal', 'FY2026', 'This document presents the proposed budget. Reconcile City-contract funding with City appropriations instead of adding both; retain later adoption and amendments separately.'),
  publication('sdhc-fy24-budget', 'SDHC FY2024 budget report', 'https://sdhc.org/wp-content/uploads/2023/05/sdhc-fy2024-budget-report.pdf', 'San Diego Housing Commission', 'Budget document', 'FY2024', 'Retain Housing Authority action and adopted revisions. Keep federal assistance, housing development, and City homelessness contracts distinct.'),
  publication('sdhc-fy25-budget', 'SDHC FY2025 proposed budget report', 'https://sdhc.org/wp-content/uploads/2024/06/har24-005-sdhc-fy25-budget-report.pdf', 'San Diego Housing Commission', 'Budget proposal', 'FY2025 · HAR24-005', 'The report requests approval; retain the Housing Authority action separately. Commission and City budget totals may include the same transfers.'),
  publication('iba-fy25-homelessness-revised', 'IBA FY2025 homelessness funding — revised', `${city}/sites/default/files/2024-09/24-24-rev-fy-2025-homelessness-programs-and-funding_rev.pdf`, 'City Independent Budget Analyst', 'Funding analysis', 'FY2025 · report 24-24 revised', 'Use the revised edition and retain revision date. Reconcile program and agency transfers before comparing with report 23-21 or later fiscal years.'),
  regionalSource('city-fy25-29-consolidated-materials', 'City FY2025–2029 consolidated-plan materials', `${city}/cdbg/general/consolidatedplanmaterials/fy25-29`, topic, 'City of San Diego',
    'The selected consolidated-plan collection and its supporting documents.', plan,
    { aliases: ['https://sandiego.gov/cdbg/general/consolidatedplanmaterials/fy25-29'], evidenceType: 'Collection', period: 'FY2025–2029', crawlPolicy: { maximumPages: 8, maximumDepth: 1, maximumResources: 100, followSupportedExternalPlatforms: false } }),
  regionalSource('csh-supportive-housing', 'Corporation for Supportive Housing', 'https://www.csh.org', 'Health and housing', 'Corporation for Supportive Housing',
    'Supportive housing research, financing resources, and published program guidance.',
    'Retain study population, geography, cost components, follow-up period, and comparison design. Research estimates do not establish local program outcomes.',
    { aliases: ['https://csh.org'], evidenceType: 'Research' }),
  regionalSource('rtfh-reports-data', 'RTFH reports and data collection', 'https://rtfhsd.org/reports-data', topic, 'Regional Task Force on Homelessness',
    'Published homelessness reports, PIT releases, HMIS summaries, and available exports.', performance,
    { evidenceType: 'Collection', crawlPolicy: { maximumPages: 12, maximumDepth: 2, maximumResources: 150, followSupportedExternalPlatforms: true } }),
  publication('sdhc-homelessness-at-a-glance', 'SDHC homelessness solutions at a glance', 'https://sdhc.org/wp-content/uploads/2022/12/sdhc-at-a-glance-homelessness-solutions.pdf', 'San Diego Housing Commission', 'Program summary', 'December 2022 path; verify document edition', 'This URL may serve a revised factsheet. Use the document’s own edition and observation dates; distinguish cumulative outcomes from annual counts.'),
  publication('state-audit-homelessness-2023-102-2', 'State audit 2023-102.2: homelessness', 'https://information.auditor.ca.gov/reports/2023-102.2/index.html', 'California State Auditor', 'Audit', 'Report 2023-102.2', 'Keep audited period, findings, agency responses, and recommendation status separate. Conclusions apply to the reviewed programs and evidence, not every homelessness program.'),
  publication('fy27-financial-summary', 'FY2027 City financial summary', `${city}/sites/default/files/2026-08/fy27ab_v1financialsummary.pdf`, 'City of San Diego', 'Adopted budget', 'FY2027', adopted),
  publication('city-homelessness-trend-announcement', 'City homelessness and sheltering trend announcement', 'https://www.insidesandiego.org/san-diego-homelessness-down-135-city-sheltering-options-and-supportive-housing-grow-91-2019', 'City of San Diego', 'Agency statement', 'Verify comparison periods in article', 'Retain each percentage’s baseline and denominator. Changes in homelessness estimates, shelter capacity, and supportive housing stock are separate measures.'),
  publication('fy25-citywide-budget', 'FY2025 Citywide budget', `${city}/sites/default/files/2024-08/fy25ab_v2citywide.pdf`, 'City of San Diego', 'Adopted budget', 'FY2025', adopted),
  publication('rtfh-hmis-dec2024', 'RTFH HMIS newsletter — December 2024', 'https://rtfhsd.org/wp-content/uploads/2025/01/rtfh-hmis-data-newsletter-dec-2024.pdf', 'Regional Task Force on Homelessness', 'Performance report', 'December 2024', `${performance} Monthly inflows and outflows do not describe all people experiencing homelessness.`),
  publication('county-draft-housing-element', 'County draft housing element — strikeout edition', `${county}/content/dam/sdc/pds/gpupdate2021/drafthousingelement-strikeoutversion.pdf`, 'County of San Diego', 'Draft plan', '2021 update path; verify planning cycle', 'Retain deleted versus proposed text and the later adoption action. County housing-element geography can be limited to the unincorporated area.'),
  publication('city-consolidated-plan-2005', 'City consolidated plan — 2005 archive', `${city}/sites/default/files/legacy/planning/community/pdf/cpc/agendas/cplan2005.pdf`, 'City of San Diego', 'Historical plan', '2005 archive', 'Confirm the document’s period and status. Historical program definitions and dollar amounts require explicit context before comparison with current budgets.'),
  publication('fy25-hssd', 'FY2025 Homelessness Strategies and Solutions', `${city}/sites/default/files/2024-08/fy25ab_v2hssd.pdf`, 'City of San Diego', 'Adopted budget', 'FY2025', adopted),
  publication('county-fifth-cycle-housing-element', 'County fifth-cycle adopted housing element', 'https://www.hcd.ca.gov/housing-elements/docs/san-diego-cou-5th-adopted050713.pdf', 'California HCD / County of San Diego', 'Historical plan', 'Fifth cycle · May 7, 2013 edition', 'Retain the County as document author and HCD as repository. Historical housing targets and sites are distinct from permits, completions, and current-cycle requirements.'),
  publication('fy27-draft-budget', 'FY2027 City Auditor draft budget', `${city}/sites/default/files/2026-06/fy27-draft-budget.pdf`, 'City Auditor', 'Budget proposal', 'FY2027', 'This is the Auditor’s budget document. Compare with adopted revisions; do not treat it as the entire City budget or as an enacted appropriation.'),
  publication('fy26-convention-center', 'FY2026 Convention Center budget', `${city}/sites/default/files/2025-08/fy26ab_v2conventioncenter.pdf`, 'City of San Diego', 'Adopted budget', 'FY2026', `${adopted} Convention Center allocations are not automatically homelessness expenditures.`),
  regionalSource('county-housing-blueprint', 'County Housing Blueprint', 'https://engage.sandiegocounty.gov/housing-blueprint', topic, 'County of San Diego',
    'Housing Blueprint documents, public engagement materials, and adopted actions.', plan,
    { evidenceType: 'Collection', crawlPolicy: { maximumPages: 8, maximumDepth: 1, maximumResources: 80, followSupportedExternalPlatforms: false } }),
  publication('rtfh-2024-pit-cities-revised', 'RTFH 2024 PIT count — revised city breakdown', 'https://rtfhsd.org/wp-content/uploads/2024/09/2024-pitc-regional-cities-breakdown-revised-final-june-17-1.pdf', 'Regional Task Force on Homelessness', 'Point-in-Time count', '2024 · revised June 17 edition', 'Retain revised city boundaries, sheltered/unsheltered definitions, and the count date. PIT is a one-night estimate; do not sum it with monthly downtown or HMIS counts.'),
  publication('escondido-meeting-housing-attachment', 'Escondido meeting attachment 002', 'https://mccmeetingspublic.blob.core.usgovcloudapi.net/escondidca-meet-4c27edfbde4e4fbe81564d7925ed98fb/item-attachment-002-2aa4e015c04f476fadae609b490f75e4.pdf', 'Escondido meeting archive', 'Meeting attachment', 'Verify meeting and item in document', 'Retain the full attachment URL, meeting identity, author, and action context. An attachment or staff recommendation is not evidence of a final Council vote.'),
];
