# San Diego homelessness accountability data: verified sources

Checked 2026-10-03. Every URL below was fetched in this session unless marked **[search-only]**.
Fetch notes: `hudexchange.info` and `huduser.gov` HTML render empty or 404 through direct WebFetch but render via `https://r.jina.ai/<url>`. RTFH and SDHC PDFs parse via r.jina.ai; direct fetch saves the binary to disk.
Labels: **documented** = stated by the source; **inferred** = my reading; **unverified** = link exists but contents were not parsed.

---

## 1. Shelter bed availability over time (camping-ordinance enforcement check)

**Verdict: a daily availability record exists inside SDHC, but nobody publishes it. No public daily or real-time series exists.**

| Source | URL | Access | Format | Frequency | Geography | Range | Covers the need? |
|---|---|---|---|---|---|---|---|
| City Auditor OCA-26-07, *Performance Audit of the City's Response to Homeless Encampments Since the Unsafe Camping Ordinance* (Apr 2026) | https://www.sandiego.gov/sites/default/files/2026-04/performance-audit-of-the-city-s-response-to-homeless-encampments-since-the-unsafe-camping-ordinance.pdf | PDF only | PDF, 7.2 MB | one-off | City shelters | FY2022–FY2025 | Annual averages only. Strongest evidence that the daily record exists. |
| SDHC / City "Homeless Services Programs Overview" dashboard | page https://sdhc.org/homelessness-solutions/city-homeless-shelters-services/dashboard/ ; embed found via WP REST https://sdhc.org/wp-json/wp/v2/pages?slug=dashboard ; metadata https://public.tableau.com/profile/api/workbook/SDHCandCityofSanDiegoHomelessServicesProgramsOverview | Tableau Public embed | Tableau | workbook `lastUpdateDate` 1789597454090 ≈ 2026-09-16 (inferred: refreshed periodically) | City of San Diego programs | unknown | Program-level counts. Views: Welcome, Shelters, TransitionalHousing, RapidRe-HousingRRH, PermanentSupportiveHousingPSH, StreetOutreach, StorageCenters, Demographics, Vulnerabilities, DataNotesDefinitions. Not daily availability (inferred from the view list; the content itself did not render). |
| City HSSD shelter page | https://www.sandiego.gov/homelessness-strategies-and-solutions/services/shelter | HTML | static list | ad hoc | per shelter | current | Static capacity per shelter (e.g. "Alpha Project Bridge Shelter I: 276 beds"). The page shows a broken "Total Beds: 0". Has no availability or occupancy. |
| RTFH HIC (annual, one January night) | https://www.rtfhsd.org/reports-data/ → e.g. https://www.rtfhsd.org/wp-content/uploads/2025/10/2025-Housing-Inventory-Count-CA-601-San-Diego-City-and-County-CoC.xlsx | XLSX download | XLSX (2025 file is 55.8 KB, one sheet; columns not parsed) | annual | CoC-wide, by project (inferred: HIC is a project-level inventory) | 2013–2025 (the "2019" link points at the 2020 file) | Capacity only, no daily availability. |
| HUD AHAR HIC by CoC | listed on https://www.huduser.gov/portal/datasets/ahar/2025-ahar-part-1-pit-estimates-of-homelessness-in-the-us.html (via r.jina.ai, published May 2026): `.../files/xls/2007-2025-HIC-Counts-by-CoC.xlsx`, raw `.../files/xls/2025-HIC-Counts-by-State.csv` (prefix `https://www.huduser.gov/portal/sites/default/files`) | XLSX/CSV | — | annual | CoC (CA-601) | 2007–2025 | Annual capacity. **Unverified**: the raw CSV fetch came back blank and r.jina.ai returned 422. Do not assume it is project-level. |
| 211 San Diego | https://211sandiego.org/housing-information/ | HTML | — | — | county | — | Only a HOPWA bed-availability link (https://211.my.site.com/s/?keyword=HOPWA%20Housing%20Board). No general shelter availability. |
| Shelter Ready app (funded by the County DA) | **[search-only]** KPBS 2025-06-09 https://www.kpbs.org/news/quality-of-life/2025/06/09/shelter-bed-app-launched-across-san-diego-county | app for outreach workers | — | real-time (per press) | county | — | Not public. |

Key verbatim findings from OCA-26-07 (read via r.jina.ai; each string confirmed by a separate presence check):
- p.25: "SDHC emails the daily shelter availability to SDPD and other shelter referral partners each morning seven days a week."
- p.24: "average occupancy rates at the City's shelters and safe sleeping sites increased from 77 percent in FY2022 and 87 percent in FY2023 to 95 percent in FY2024 and FY2025."
- p.30: "The first contact for all three violations results in a verbal warning, which officers document as a field interview."
- Auditor's summary (paraphrased by the summarizer, not a verbatim quote): enforcement records do not systematically capture whether shelter was available at the time of contact. The audit makes no data recommendation on this.

Public-records trail (City NextRequest JSON API, `https://sandiego.nextrequest.com/client/requests?search_term=...`):
- **23-4022** (2023-07-20, HSSD, closed): "A spreadsheet of daily available shelter beds from January 1, 2023, through July 20, 2023" with bed locations. The requester says the San Diego Homelessness Resource Center receives these numbers daily from the city. https://sandiego.nextrequest.com/client/requests/23-4022
- **23-5615** (2023-10-05, closed): daily shelter bed availability at City shelters, including safe sleeping and safe parking, from 2023-01-01.
- **22-2329** (2022-05-03, closed): shelter beds and occupancy, 2019 to present.
- A documents search for "availability" returned 0 shelter documents. Released files, if any, are not publicly visible.
- **Practical route:** file a new CPRA request to SDHC and HSSD for the archive of the daily availability emails, citing OCA-26-07 p.25. Then join it to SDPD enforcement data (enforcement data was not researched here).

## 2. HMIS outcomes (exits to PH, returns, length of stay, cost per client) by provider / ZIP / council district

**Verdict: machine-readable outcome data is CoC-level only. Provider-level outcomes exist only in RTFH's Tableau dashboard, and its export is unverified. Nothing public is broken down by ZIP or council district. Nothing public reports cost per client.**

| Source | URL | Access | Format | Frequency | Geography | Range | Covers the need? |
|---|---|---|---|---|---|---|---|
| Cal ICH CA System Performance Measures (from HDIS) | https://data.ca.gov/api/3/action/package_show?id=ca-system-performance-measures-statewide-and-by-coc ; CSV https://data.ca.gov/dataset/d3f9ca4d-3e60-434b-9fa3-91ee0befd34d/resource/e02178d9-1d34-4798-9979-f50af9f1742e/download/ca-spms-prior-years-and-last-twelve-months-cy2025.csv ; dictionary `.../fd4ff7c8-a54a-4af2-9a13-03a8fdcc9b22/download/ca-spms-data-dictionary.csv` | **CKAN datastore API works**: `https://data.ca.gov/api/3/action/datastore_search?resource_id=e02178d9-1d34-4798-9979-f50af9f1742e&q=San%20Diego` | CSV / JSON | metadata "R/P3M" (quarterly); modified 2026-08-04 | CoC (CA-601) | CY2020–CY2025 | **Best machine-readable outcome series.** Fields: Location, Metric (M1b, M2–M5…), one column per calendar year. Example: CA-601 M4 = 109,111,125,129,138,144. Check the dictionary for metric meanings. CoC-level only. |
| Cal ICH demographics | https://data.ca.gov/api/3/action/organization_show?id=california-interagency-council-on-homelessness (dataset `homelessness-demographics`, modified 2026-08-17; also `help-act-goals-statewide-and-by-coc`) | CKAN | CSV | — | CoC | — | Demographics, not outcomes. |
| HUD SPM data since FY2015 | page https://www.hudexchange.info/resource/5691/system-performance-measures-data-since-fy-2015/ (r.jina.ai; updated Jan 2026) → https://files.hudexchange.info/resources/documents/System-Performance-Measures-Data.xlsx (fetched: 2.4 MB XLSX, columns not parsed) | XLSX | XLSX | annual | CoC | FY2015+ | Length of time homeless, returns, exits to PH. CoC-level. |
| RTFH SPM PDFs | https://www.rtfhsd.org/reports-data/ (e.g. https://www.rtfhsd.org/wp-content/uploads/2025/01/SPM-FY2022.pdf) | PDF only | PDF | annual | CoC | FY2017–FY2022 only | Superseded by the HUD XLSX for later years. |
| RTFH LSA / Stella P reports | same page (e.g. https://www.rtfhsd.org/wp-content/uploads/2024/10/CA-601-FY23-LSA-CA-601-v4-print_all.pdf) | PDF only | PDF | annual | CoC, by household type / pathway | FY2019–FY2023 | Length of stay and exits by system pathway. Not by provider. Nothing after FY23 is posted. |
| RTFH annual Data & Performance Reports | e.g. https://www.rtfhsd.org/wp-content/uploads/2026/07/2025-Data-Performance-Report_Email.pdf | PDF only | PDF | annual | CoC | 2021–2025 | Narrative system metrics (contents not parsed). |
| RTFH HMIS monthly newsletter | e.g. https://www.rtfhsd.org/wp-content/uploads/2026/09/HMIS-Data-Newsletter-August-2026.pdf (all links on /reports-data/) | PDF only | 2-page PDF | monthly | **system-wide only** | Sep 2022–Aug 2026 | Sections: "WHAT'S NEW", "WHO'S ACTIVE?", "WHO FOUND HOUSING?" (Aug 2026: 884 persons housed), "HMIS SUPPORT STATS". Also covers referrals to the housing queue. No provider, city, or ZIP breakdown. |
| RTFH 12-month overviews; veterans monthly reports | /reports-data/ | PDF only | PDF | annual / monthly | system-wide | FY22–FY25; Jan 2025–Aug 2026 | Same limits. |
| RTFH project-level HMIS dashboard | https://public.tableau.com/app/profile/hmis.team/viz/CA-601Dashboard_17792177078930/DataDashboard ; metadata https://public.tableau.com/profile/api/workbook/CA-601Dashboard_17792177078930 | Tableau Public embed | Tableau | `lastUpdateDate` 1784579046151 ≈ 2026-07-20 | **project level** (per RTFH: "5 years of HMIS data") | ~5 yrs | **The only provider/project-level outcome source.** Views include DataDashboard, SystemFlowDashboard, NumberofClientsThatReappearWithoutStableHousingbreakdown, DataDashboardFY2026, FilteredView. `.csv` and `.png` view exports returned 404 through WebFetch, so export is **unverified, not proven absent**. Next check: run curl or the `tableauscraper` Python library (VizQL bootstrapSession) from a real HTTP client. |
| homelessdata.com RTFH dashboards (SPM, project performance, community, YHDP) | https://www.homelessdata.com/rtfh-landing-page/ ; https://homelessdata.com/dashboard/rtfh/annual/ | — | — | — | — | — | **Dead.** Verbatim: "The RTFH has decided to no longer support these performance monitoring dashboards, we apologize for the inconvenience." |
| RTFH data request form | https://www.rtfhsd.org/wp-content/uploads/2025/10/Data-Request-Fillable-Form-April-2023.pdf ; support@RTFHSD.org | request | PDF form | — | — | — | Route to custom extracts (ZIP / provider). |

## 3. Coordinated Entry match / placement timelines; HCV / EHV issuance vs lease-up

**Verdict: no public CES timeline data exists. The EHV/HCV data sits in HUD Power BI dashboards with no download. Annual PHA-level voucher data is downloadable from HUD's Picture of Subsidized Households.**

| Source | URL | Access | Format | Frequency | Geography | Range | Covers the need? |
|---|---|---|---|---|---|---|---|
| RTFH CES page | https://www.rtfhsd.org/about-coc/coordinated-entry-system-ces/ | HTML | — | — | — | — | Training and policy only. No data. |
| RTFH HMIS newsletter "referrals to the housing queue" | see §2 | PDF | — | monthly | system | 2022– | Counts only. No time-to-match or time-to-move-in. |
| RTFH monthly veteran housing reports | /reports-data/ (2023 files are titled "BFZ-30-Day-move-in-date-calculator", e.g. https://www.rtfhsd.org/wp-content/uploads/2024/10/BFZ-30-Day-move-in-date-calculator-September-2023.pdf) | PDF | PDF | monthly | veterans, system | Jul 2023–Aug 2026 | Partial: placements and possibly move-in timing for veterans only (contents not parsed). |
| HUD EHV dashboard | https://www.hud.gov/helping-americans/housing-choice-vouchers-emergency-dash ; Power BI https://app.powerbigov.us/view?r=eyJrIjoiYjU4MzlkNzEtM2MxZi00ZjhjLTkyNTEtYjI2OWUzZjA0YTIwIiwidCI6IjYxNTUyNGM1LTIyZTktNGJjZC1hODkzLTExODBhNTNmYzdiMiJ9 ; awards XLSX https://www.hud.gov/sites/dfiles/PIH/documents/Emergency_Housing_Voucher_Awards.xlsx (2022-04-18); dictionary https://www.hud.gov/sites/dfiles/PIH/documents/EHV_Dashboard_UserGuide_Data_Dictionary.pdf | Power BI embed, no download, no stable API | — | "refreshed daily" from IMS/PIC and HUDCAPS | PHA (SDHC) | 2021– | Shows current issued vs leased. **No historical series to download.** You would need to scrape and snapshot it yourself. |
| HUD HCV dashboard | https://www.hud.gov/helping-americans/public-indian-housing-hcv-dashboard ; dictionary https://www.hud.gov/sites/dfiles/PIH/documents/Dashboard-Data-Dictionary-FINAL.pdf | Power BI embed, no download | — | monthly, ~2-month lag | PHA | — | Leasing, budget, per-unit cost. No export. |
| HUD Picture of Subsidized Households | https://www.huduser.gov/portal/datasets/assthsg.html (via r.jina.ai); API https://www.huduser.gov/portal/dataset/picture-api.html | Excel + API | XLSX / JSON | annual | PHA, project, tract, ZIP, congressional district | 2009–2025 | Annual voucher counts and occupancy. Not issuance-to-lease timing. |
| SDHC Board reports | https://sdhc.org/governance-legislative-affairs/sdhc-board-of-commissioners/meetings/ (e.g. agenda https://sdhc.org/09112026-sdhc-regular-meeting-agenda/) | PDF only | agenda PDFs with HCR-numbered staff reports, plus YouTube video | ~monthly | SDHC | 2026 on the page, older years under "Archives" | EHV and HCV status appears in staff reports and presentations. **[search-only]** claims: SDHC received 501 EHVs, and funding lasts through fall 2026. |

## 4. Sub-recipient contracts and budgets; admin vs direct-service split

**Verdict: award lists exist, mostly as PDFs. No public source gives an admin vs direct-service split per sub-recipient.**

| Source | URL | Access | Format | Frequency | Geography | Range | Covers the need? |
|---|---|---|---|---|---|---|---|
| HUD FY2024 CoC awards, California | https://www.hud.gov/sites/dfiles/CPD/documents/CoC/CoC-2024-CA_Press.pdf (via r.jina.ai) | PDF only | PDF | annual | per project | FY2024 | CA-601 total "$44,053,677". Columns: "CoC Name, Program, Awarded Amount, Project Name". Project-level amounts (e.g. "Alpha Square 2024 (FY25-26) — $603,893"). No recipient-organization column was seen, and no admin split. FY2025 round 1 **[search-only]**: https://www.hud.gov/sites/default/files/OCHCO/documents/ALL-Press-Round-1.pdf |
| RTFH Grants & Contracts | https://www.rtfhsd.org/funding/grants-recipients/ | HTML + PDF | PDF | per competition | per org / project | 2019– | 2026 HHAP Strategic Investments final posting https://www.rtfhsd.org/wp-content/uploads/2026/10/2026_HHAP_Strategic_Investment_Final_Award_Posting_10.1.26.pdf. Columns: "Applicant Organization, Proposal Title, Requested Amount"; 30 applications; "Total Requested $38,031,407.45". These are **requested** amounts; final awards are subject to negotiation. Also posted: preliminary results https://www.rtfhsd.org/wp-content/uploads/2026/04/2026-HHAP_RFP_Preliminary-Results.pdf ; Super NOFO 2023 awardees https://www.rtfhsd.org/wp-content/uploads/2024/09/RTFH-SNOFO-Awardees_FINAL_4_9_2024.pdf ; YHDP letters. No admin split. |
| HHAP-6 Regional Application (CoC + City + County) | https://www.rtfhsd.org/wp-content/uploads/2025/09/HHAP-6-Regional-Application-San-Diego-City-and-County-CoC-UID-57-updated-submission-10.29.2025-1.pdf | PDF only | 285 pp | per round | CoC, City, County | HHAP 6 | Confirms City, County, and CoC each contract separately with the state (p.3). **Unverified:** the r.jina.ai read stopped at p.45, before any budget-by-eligible-use table. The HHAP funding plan normally has an administrative line (inferred). Read the later pages from disk. |
| CA HCD HHAP fiscal dashboard | https://www.hcd.ca.gov/planning-and-community-development/housing-open-data-tools/hhap-data-dashboard-and-downloads | embedded dashboard; embed URL not exposed in HTML, even via r.jina.ai | — | grantees self-report monthly, cumulative per round (documented) | per grantee (City, County, CoC); per sub-recipient not stated | HHAP rounds | Obligations and expenditures by statutory eligible use. The admin category exists only at **grantee** level, if at all. No downloadable file found; data.ca.gov search for "HHAP" returned 0 datasets. Template: https://www.hcd.ca.gov/sites/default/files/docs/grants-and-funding/calich/hhap-annual-report-template-2026.pdf **[search-only]** |
| City Council agendas and staff reports | https://sandiego.hylandcloud.com/211agendaonlinecouncil (OnBase Agenda Online; pattern `/Meetings/ViewMeeting?id=<n>&doctype=1|3&site=council`) | HTML + PDF | PDF | per meeting | City | through 2026-10 | Contract approvals and HSSD staff reports. PDF only. San Diego does **not** use Legistar; `webapi.legistar.com/v1/sandiego/*` returns HTTP 500. |
| City contracts via CPRA | NextRequest 24-3385 (the City's current homeless-services contracts, recorded in prior research) | PRA | — | — | — | — | Files are not openable without logging in. |
| SDHC Board | §3 | PDF | — | — | — | — | Shelter-operator agreements and budgets appear as HCR staff reports. |

## 5. Point-in-Time count by area

| Source | URL | Access | Format | Frequency | Geography | Range | Covers the need? |
|---|---|---|---|---|---|---|---|
| RTFH WeAllCount, City of San Diego | e.g. https://www.rtfhsd.org/wp-content/uploads/2026/05/2026-PIT-City-of-San-Diego_Final.pdf | PDF only | PDF | annual (January) | city, with in-city breakdown | 2014–2026 | Yes, as PDFs. |
| RTFH unsheltered count by census tract | 2026 https://www.rtfhsd.org/wp-content/uploads/2026/05/2026-PITC-Unsheltered-Census-Tract-per-City.xlsx (fetched: 18.5 KB XLSX, columns not parsed); 2025 https://www.rtfhsd.org/wp-content/uploads/2025/08/Unsheltered-Clients-Census-Tract-Count-2025-PITC.xlsx ; 2024 https://www.rtfhsd.org/wp-content/uploads/2025/07/Client-Count-by-2020-Census-Tract-and-Cities.xlsx | XLSX | XLSX | annual | census tract × city | 2024–2026 (OCA-26-07: tract data "not available for 2022 and 2023") | **Best machine-readable sub-city PIT.** Unsheltered only. Council-district figures require a spatial join to district boundaries. |
| RTFH 2023 PITC summary | https://www.rtfhsd.org/wp-content/uploads/2025/01/PITC-Summary_2023_All.xls | XLS | XLS | — | sheltered + unsheltered | 2023 | — |
| HUD PIT by CoC | `https://www.huduser.gov/portal/sites/default/files/xls/2007-2025-PIT-Counts-by-CoC.xlsb` (listed on the AHAR page; not fetched) | XLSB | XLSB | annual | CoC | 2007–2025 | CoC-level only. |
| Downtown San Diego Partnership monthly sleep counts | prior research: UCSD H-Hub ArcGIS `Downtown_Partnership_January_2014_June_2025_Points_WFL1/FeatureServer/0` | ArcGIS REST | JSON | monthly | downtown neighborhoods | 2014–Jun 2025 | Only monthly sub-city series (downtown only). |

---

## Needs with NO public source

1. **Daily or historical shelter bed availability.** SDHC produces it every morning (OCA-26-07 p.25) and never publishes it. PRA requests 23-4022 and 23-5615 sought exactly this, and no released records are visible. Real-time availability (Shelter Ready) is limited to outreach workers.
2. **Shelter availability at the moment of each enforcement contact.** Per the auditor, SDPD records do not systematically capture it. Even with the daily SDHC file, the join would be by day, not by contact.
3. **HMIS outcomes by ZIP code or council district.** None anywhere. Every machine-readable outcome source (Cal ICH SPM CSV, HUD SPM XLSX) is CoC-level. Provider-level exists only in RTFH's Tableau, where export is unverified.
4. **Cost per client / cost per outcome by provider.** No public dataset. You would have to assemble it from award PDFs (HUD CoC, RTFH, Council/SDHC staff reports) divided by HMIS counts, and the two don't share a provider key.
5. **CES match and placement timelines** (time from assessment to match to move-in). No public source. Only aggregate monthly queue-referral counts, plus veteran-only move-in reports.
6. **EHV / HCV issuance-to-lease time series.** HUD shows only the current snapshot in Power BI. Historical series require self-archiving, or SDHC board PDFs.
7. **Admin vs direct-service cost split per sub-recipient.** Not published by HUD, RTFH, HCD, or the City. At best, a grantee-level admin line in HHAP funding plans (HHAP-6 PDF, unverified) and in individual contract budgets obtainable via CPRA.
8. **Recent system reports from RTFH.** LSA/Stella stops at FY2023 and RTFH-hosted SPM PDFs stop at FY2022. Use the HUD SPM XLSX and the Cal ICH CSV for later years.
9. **Council-district PIT.** Not published. Derivable only by joining the census-tract XLSX (2024–2026) to district boundaries.
