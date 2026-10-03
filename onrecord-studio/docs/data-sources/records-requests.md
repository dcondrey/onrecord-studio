# City of San Diego records: already released vs. must be requested

Research date: 2026-10-03. All request data comes from the City's NextRequest JSON API (`sandiego.nextrequest.com`), searched 2026-10-03.

**Method.**
- `client/requests?search_term=` finds candidate requests.
- `client/request_documents?request_id=<id>` gives the authoritative list of released file titles.
- `client/requests/<id>/timeline?page_number=1` gives the closure code, which is how released, withheld, referred, and no-records outcomes are told apart. Every hit says `request_state: "Closed"`, so that field doesn't distinguish them.

**Limits.**
- Request descriptions below are paraphrased by the fetch summarizer unless shown in quotation marks with "(verbatim)".
- The `documents` search matches file titles only. It missed the 25-6107 xlsx, for example.
- Search is fuzzy on multi-word terms, so totals such as "notice" (3010 hits) are mostly noise.
- Released files can't be opened without authentication. Contents are known only from file titles plus the request scope.
- Some requests closed as "Released" show **0 public documents**: 26-1368 and 25-6154. Those files went only to the requester, or were never published.

---

## Part A: What has already been released

### A1. SDMC 63.0404 (Unsafe Camping) citations and enforcement: WELL COVERED through mid-2025

| Request | Asked for | Closure | Released files |
|---|---|---|---|
| **25-6107** (Police, filed 2025-08-11, San Diego Union-Tribune) | Spreadsheet of all citations/arrests under MC 54.0110, 63.0404, and PC 647 from 2021-01-01 to production, with activity date, violation section, person details, and address | `02c. Released – Redacted and/or Withheld` (juvenile info, W&I 827), 2025-09-19 | `PRA_25-6107_Arrestee Log_01012021-07112025_Anonymized_SDUT_Taketa_SDPD.xlsx`; `PRA_25-6107_Arrestee Log with Location_07122025-08112025_SDUT_Taketa_SDPD.pdf` |
| **25-7564** (Police, 2025-09-28) | All arrests and citations under SDMC 63.0404, 2023-07-31 to 2025-07-31 | `02. Released`, 2025-10-09 | `PRA_25-7564_All_ArretsCites_Kami_Leone_SDPD.xlsx` |
| **25-9260** (Police, 2025-11-20, UCSD US Immigration Policy Center) | Electronic spreadsheet of citations/arrests of unhoused people, 2023-06-28 to 2025-06-28, with dates, times, violation codes, demographics | documents released | `PRA 25-9260_ArrestsCites_SDPD.xlsx` (2025-11-28); `PRA 25-9260_06-28-25_SDPD.xlsx` (2026-03-18) |
| **ACLU-SDIC monthly series** (Police): 24-4461, 24-5096, 24-6043, 24-6702, 24-7462, 24-8218, 24-8905, 25-906, 25-1507, 25-2437, 25-3052, 25-3970, **25-4847** (last one, 2025-05-25 to 2025-06-25) | Arrests, citations, and warnings under a list of ordinances (44.0103, 54.0110, 63.0404, etc.), with officer, demographics, location, and jail/release | e.g. 25-4847 released | `PRA_25-4847_ACLU_SDPD.pdf` (PDF only). The series stops after June 2025. |
| 25-3267, 24-1325, 23-7275, 23-6364, 23-4812, 24-6575 (Police) | Earlier 63.0404 data pulls | Closed | not individually checked |
| **24-1883, 24-5534, 25-6105** (City Attorney) | Prosecutions/case referrals for 63.0404, 54.0110, PC 647(e) | 24-5534 released `SDMC63_0404.pdf` | 25-6105 is the 2021-onward spreadsheet version (documents not checked) |
| **23-5873 / 24-6756** (Police) | SDPD training bulletin on 63.0404 | Released | `Enforcement of SD Municipal Codes 63.0403 & 63.0404 (Redacted)` + `Attachment A` (2023-10-24; re-released 2024-10-16). 23-4216 and 23-5086 came earlier and were told the bulletin did not exist yet. |
| **24-8220** (Environmental Services, ACLU) | ESD staff time, salaries, and expenditures for enforcing these ordinances, last 2 fiscal years | Released | `PRA #24-8220 - ESD - Combined Files.pdf` |

**Gap:** No released record-level 63.0404 data after **2025-08-11** (the 25-6107 PDF). Nothing found in native format after **2025-07-31** (25-7564).

Caveat on completeness: the `63.0404` search reports `total_count` 36, but only 30 ids were returned via both the summarizer and r.jina.ai, and page 2 is empty. Six requests are unseen. Follow-up "camping" and "unhoused" sweeps limited to requests filed after 2025-08-01 found no later SDPD/ESD/City Attorney data request. So the gap is probable, not proven.

**Aggregate already public:** City Auditor report **OCA-26-07** (April 2026), "Performance Audit of the City's Response to Homeless Encampments Since the Unsafe Camping Ordinance" ([highlights PDF](https://www.sandiego.gov/sites/default/files/2026-04/unsafe-camping-ordinance-highlights.pdf)).
- Exhibit 13 gives SDPD field interviews, citations, and arrests for the Ordinance, encroachment, and illegal lodging by fiscal year. FY2025: 4,730 FI / 1,605 citations / 391 arrests.
- Source line: "data SDPD provided from the Automated Regional Justice Information System (ARJIS)".
- It also reports ESD and SDPD response times to Get It Done encampment reports.
- **Field interviews** outnumber citations about 3:1, and no PRA release found covers FI-level records. Add FIs to B1.

**Ordinance numbering** (SDMC Ch. 6 Art. 3 Div. 4, [municode PDF dated 7-2023](https://docs.sandiego.gov/municode/MuniCodeChapter06/Ch06Art03Division04.pdf); added by O-21674 N.S., eff. 2023-07-29):
- §63.0401 Purpose; §63.0402 Definitions; §63.0403 Protection of Waterways; **§63.0404 Unauthorized Encampments on Public Property** (the offense); §63.0405 Enforcement; §63.0406 Abatement of Encampments.
- 26-1367/26-1368 cite "63.0401" for the offense. That is the Purpose section, a requester error.
- 25-9123 (City Clerk, 2025-11-16) mentions ordinance O-21696 N.S. alongside O-19670. Whether O-21696 amended Div. 4 was not checked.

**Wording that worked:** "spreadsheet of all citations and arrests ... including activity date, ... violation section ... address of arrest/citation." SDPD's own name for the export is **"Arrestee Log"**, produced as **xlsx**.

### A2. NPD (Neighborhood Policing Division): deployment, activity logs, citations. PARTIAL and STALE

| Request | Asked for | Closure | Released files |
|---|---|---|---|
| **22-5840** (Police, 2022-11-03) | Quoted below | Released | `SDPD Recap for all squads 040119 to 103122.pdf` |
| **22-4629** (Police, 2022-08-31) | NPD officer counts FY18–22; HOT officers/supervisors; NPD overtime-hour reports from Dec 2020 | `02. Released` | `NPD OT FY21 P6 - FY23 P2 09.07.22.pdf` + adopted-budget excerpts `fy18ab_v2police.pdf` … `fy23ab_v2police.pdf` |
| **22-3937** (2022-07-25) | FY22/23 budgeted staffing and actual expenses for Neighborhood Policing Management/Teams and the Homeless Outreach Unit | Closed | not checked |
| **25-6154** (Police, 2025-08-12) | List of all staff assigned to NPD, with job titles | **`01a. Not Released - Withheld`**, §7923.600 + §7922.000 (2025-08-21) | none |
| **26-1368** (Police, 2026-02-07) | CAD reports, dispatch narratives, BWC metadata, and NPD guidance for 4101 W Point Loma Blvd/Chapman St, 2024-12-01 to 2026-02-07 | `02c. Released – Redacted and/or Withheld`, §7923.600 (2026-03-18) | **0 public documents** |
| 26-1367 (Mayor, 2026-02-07) | Mayor's-office communications on "no tent" enforcement criteria and guidance to NPD | Closed | not checked |

22-5840 asked for (verbatim): "Neighborhood Policing Division's "Recap for all squads" reports from July 1, 2016, through November 3, 2022."

The 22-5840 release starts at 2019-04-01, not 2016. Inferred: earlier recaps were not retained.

**Gap:** NPD squad recaps after **2022-10-31** and NPD overtime after FY23 P2 have never been released publicly. An NPD roster is withheld outright, so don't ask for one.

### A3. ESD encampment abatement logs, 24-hour notices, notice postings: WELL COVERED for abatements, NOT for individual notices

| Request | Asked for | Closure | Released files |
|---|---|---|---|
| **26-772** (ESD, 2026-01-20) | Columns quoted below | `02. Released`, 2026-01-30 (10 days) | `Copy of Abatements Completed 7.31.23 to 1.28.26.xlsx` |
| **25-360** (ESD + Police, 2025-01-15, CalMatters) | Every sweep since 2024-01-01: location, etc. | `02. Released`, 2025-02-07 | `PRA #25-360 - ESD - Encampments Abated Report 1.31.2025.pdf`; `... FINAL Encampment Abatement Procedures 8.8.23.pdf`; `... Ch06Art03Division04.pdf`; `... Isaiah Settlement.pdf`; `... Impounded Items Report 1.31.2025.pdf`; `... CY 2024 Encampment Abatements 1.31.2025.pdf`; `PRA 25-360_Marisa Kendall_CalMatters.xlsx` |
| **24-1587** (ESD, 2024-02-29) | Downtown encampment removals 2015–2024, plus "a list of all the encampment removal notices" | `02. Released`, 2024-05-01 | `PRA #24-1587 - ESD Downtown Abatements Feb 2019 to Mar 2024.pdf`. No notice list appears among the released files. |
| **23-4703, 23-3689, 23-1966** (ESD) | "breakdown of CleanSD ... postings of notices of intent to abate encampments ... three, 24-hour and/or 72-hour timeframe" | Released | Monthly `Clean SD Summary` PDFs only |
| **Clean SD Summary** monthly series (~85 files) | Monthly ESD summary that, per 23-4703's description, carries posted and abated encampment totals | — | Covers 2019 to **April 2026**. Latest files: `PRA #26-4237 - ESD - Clean SD Summary April 2026.pdf` (2026-05-19), `Clean SD Summary March 2026.pdf` (26-3143), `PRA #26-2054 ... February 2026`, `PRA #26-1101 ... January 2026`, `PRA #26-79 ... December 2025` |
| 23-45 (ESD, 2023-01-04) | Full cost accounting of encampment abatements FY17–present (ESD + NPD) | Closed | not checked |
| 25-8547 (ESD, 2025-10-29) | Urban Corps encampment clean-up documents and contracts | Closed | not checked |

26-772 asked for these columns (verbatim): "date and time of abatement, abatement address, abatement geocoordinates (if available), number of people present at encampment at time of abatement (if available), tonnage of items disposed (if available)".

The ordinance's own record names (SDMC §63.0406):
- a written **"Notice of Clean-Up"** posted at each tent, giving a minimum of 24 hours (reposted if abatement slips more than 48 hours);
- an **"Impound Notice"** when belongings are stored;
- mandatory photo/video documentation before, during, and after abatement, and of all items stored (§63.0406(c)(1)–(6));
- property of a person arrested at an abatement goes "to the San Diego Police Department for impoundment" (§63.0406(b)(3)). That is a second, SDPD-side impound stream.

**Gaps:**
- Abatement row-level data after **2026-01-28**.
- Clean SD Summaries for **May–Sept 2026**: no release found.
- No released **per-posting notice log** (address, date/time posted, notice type 3/24/72-hour, abatement date) for any period. Requesters who asked for notices received summary counts only.

### A4. Impounded property / storage intake logs: COVERED to ~mid-2024, one redacted response to 2025

| Request | Asked for | Closure | Released files |
|---|---|---|---|
| **24-4229** (ESD, 2024-06-18, media) | Impound records | Released 2024-07-15 | `PRA #24-4229 - ESD - Abatement Impound Log 7.1.22 to 10.4.23.pdf`; `PRA #24-4229 - ESD - Abatement Impound Logs 10.1.23 to Current.pdf` |
| **25-7303** (ESD, 2025-09-19) | Quoted below | `02a. Released - Redacted`, §7922.000 personal identifying info (2025-10-27) | `PRA - 7303 - ESD - Response for 25-7303 Final.pdf` (single PDF) |
| **25-360** | (see A3) | Released | `PRA #25-360 - ESD - Impounded Items Report 1.31.2025.pdf` |
| **23-3164** (ESD, 2023-06-09) | "records of all personal property collected and logged during encampment cleanings" 2019–2022 | `02. Released` 2024-01-03 | ~400 JPG/TimePhoto images plus `PRA #3164 ESD - Clean SD Summary Reports.pdf`, i.e. photos, not a structured log |

25-7303 asked for (verbatim): "impound logs covering Jan. 1, 2023 to the date of production of records that maintain specific descriptions and/or photos of items that have been impounded during homeless encampment abatements. Please include log(s) with at least ... date impounded, impounded location, description of impounded items, status (i.e. disposed, retrieved by owner, etc)".

Searches for "storage center", "Transitional Storage", and "Think Dignity" found **no** requests about a storage-facility intake/retrieval manifest.

**Gaps:**
- Native (xlsx) impound log. Everything released is PDF.
- Period after 25-7303's production (2025-10).
- Storage-facility-side retrieval/disposal records.

The City's record names are **"Abatement Impound Log"** and **"Impounded Items Report"**.

### A5. Shelter bed availability / daily bed counts: City says it DOES NOT HOLD them

| Request | Agency | Asked for | Closure |
|---|---|---|---|
| **22-2329** (2022-05-03) | City / HSSD | Total shelter beds and beds occupied daily, 2019–present | **Referred to another agency**: "The City of San Diego does not maintain the records requested." |
| **23-4022** (2023-07-20) | City / HSSD | Spreadsheet of daily available shelter beds, 2023-01-01 to 2023-07-20 | **Referred to another agency** (same text) |
| **23-5615** (2023-10-05) | City / HSSD | Daily bed availability at City shelters (safe sleeping, safe parking, others) | `02. Released` 2024-01-19: `SafeSleeping_2023.pdf`, `ES_2023.pdf`, `ES_FY25.pdf` (contents unknown; titles suggest per-program reports) |
| 23-5611 (2023-10-05) | City / HSSD | People served and outcomes for City-contracted shelters, monthly provider reports | not checked |
| **26-7360** (2026-07-24) | City / HSSD | Father Joe's Villages monthly bed-usage/occupancy reports submitted to HSSD | **No responsive documents / Referred to another agency** (2026-08-03) |
| **SDHC 26-145** (2026-07-23) | SDHC | Father Joe's/Mirabile Center monthly bed count and occupancy | "SDHC does not have records responsive to your request." (same day) |

The SDHC portal (`sandiegohousingcommission.nextrequest.com`, same API shape) had no request for Coordinated Shelter Intake daily bed availability. It returned 11 "shelter" and 36 "bed" hits.

SDHC states it "administers shelter, transitional housing and supportive services programs on behalf of the City through several Memoranda of Understanding" and "oversees a coordinated shelter intake process to maximize use of available beds within the system" ([sdhc.org](https://sdhc.org/homelessness-solutions/city-homeless-shelters-services/)). The City's intake page names HMIS as the tracking system ([sandiego.gov](https://www.sandiego.gov/homelessness-strategies-and-solutions/services/coordinated-intake-system)).

**Inferred:**
- Daily bed availability for City-funded shelters sits with SDHC's Coordinated Shelter Intake and/or in HMIS.
- HMIS is administered by the Regional Task Force on Homelessness, a nonprofit probably outside the CPRA.
- The Father Joe's/Mirabile request probably failed because it named a program that neither the City nor SDHC funds.

The SDHC dashboard page (`/dashboard/`) is JS-rendered and returned no content via fetch.

### A6. SDPD overtime by event or division: payroll-level only; event level not released

| Request | Asked for | Released files |
|---|---|---|
| **24-6535** (Police, 2024-09-23) | Annual payroll 2021–24 incl. overtime, plus "incident-level data showing overtime instances by date, location, type" | `24-6535 Compensation Report CY2021-CY2023_SDPDSwornOnly.xlsx`; `24-6535 - Labor Detail - Summary (By Employee) Sworn Positions Only Jan-Sept 2024 as of 10.28.24_FINAL.xlsx`. No incident-level file among the titles. |
| **25-2736** (2025-04-08) | Same ask | Closed by pointing to 24-6535 |
| **22-4629** | NPD overtime | `NPD OT FY21 P6 - FY23 P2 09.07.22.pdf`, overtime by division by pay period, which is the format to ask for again |
| 22-3442 | — | `Items #3 and #4 Labor Detail - Overtime_ran 07.13.22.pdf` |
| **25-274** (2025) | Special-event police costs (San Diego Rodeo / Petco Park) | `2024 Signed Final San Diego Rodeo Cost Estimate_Redacted.pdf`, `Final Invoice_Redacted.pdf`, `City Master Recon as of 6.12.24.xlsx`, `City of SD Share Calculation ...pdf` |
| 26-4074 (2026-04-27) | Overtime cost per officer, past calendar year | not checked |

**Gap:** Overtime by division after FY23 P2, and overtime by event/activity code. The 25-274 release suggests per-event cost estimates and invoices exist for cost-recoverable special events (inferred from titles).

### A7. Calls for service / CAD data: PUBLISHED as open data (minus sensitive types and unit/officer fields)

- `data.sandiego.gov/datasets/police-calls-for-service/`: 2015–2026, updated daily (last update 2026-10-03).
  - Fields: incident number, date/time, address parts, call type, disposition, beat, priority.
  - Excludes "domestic violence, child abuse, suicide, sex crimes and stalking."
  - CSV: `https://seshat.datasd.org/police_calls_for_service/pd_calls_for_service_2026_datasd.csv` (and `_2025_`).
- **22-1579** (2022-03-25) asked for every CFS 2016–2021 with the responding-officer identifier and demographics. The City pointed to the open-data page and closed with `02b. Released - Withheld` under former §6254(f) (now §7923.600). **Officer/unit identifiers were withheld.**
- **26-1368**: location-specific CAD for an encampment area; released only to the requester (0 public files).
- **26-7968** (2026-08-10) asked for all 2021 CAD/CFS records. Not checked.

**Gap:** The open data has no responding unit/division, so NPD/HOT-handled calls can't be isolated. It also has no dispatch/arrival/clear timestamps, and no call-type subset tied to encampments.

### A8. Fee / cost estimates for records: no prior requests found; policy is documented

Searches for deposit, invoice, estimate, and "7922.575" found no request about PRA fee estimates. "estimate" hits are construction and event cost estimates.

Policy:
- **City:** "Fees for copies are $.25 per page ... There is no charge for electronic records that are provided via the public records portal." ([sandiego.gov PRA page](https://www.sandiego.gov/communications/public-records-requests))
- **City AR 95.21 §5.3.8** (Issue 2, eff. 2022-11-22): "If fulfilling the Request requires the construction of a record that entails data compilation, extraction, or programming, the requester must bear the cost of production ... PRA Program staff or PRA Liaison must obtain written authorization from the requester before creating a record at the cost of the requester." ([AR 95.21 PDF](https://www.sandiego.gov/sites/default/files/ar9521.pdf))
- **SDHC:** 15¢/page single-sided, 25¢ double-sided, "no charge for electronic or PDF files that are emailed." ([sdhc.org](https://sdhc.org/about-us/public-records-request/))

**Practical consequence:** The timelines read for 25-6107, 25-7564, 26-772, and 24-6535 show no fee estimate or invoice entry before release. Inferred: the City treats routine report runs from its systems as existing records rather than §7922.575(b)(2) programming. Ask for the export a system already generates, and pre-authorize a fee cap so a cost estimate doesn't stall the request.

---

## Part B: What must be requested

### Statutory frame (verified 2026-10-03 from leginfo via r.jina.ai)

**Gov. Code §7922.535(a)**, 10 days (verbatim): "Each agency, upon a request for a copy of records, shall, within 10 days from receipt of the request, determine whether the request, in whole or in part, seeks copies of disclosable public records in the possession of the agency and shall promptly notify the person making the request of the determination and the reasons therefor. If the agency determines that the request seeks disclosable public records, the agency shall also state the estimated date and time when the records will be made available."

**§7922.535(b)**, extension (verbatim): "In unusual circumstances, the time limit prescribed in this article and Article 1 (commencing with Section 7922.500) may be extended by written notice from the head of the agency or a designee to the person making the request, setting forth the reasons for the extension and the date on which a determination is expected to be dispatched. The notice shall not specify a date that would result in an extension for more than 14 days."

A valid extension requires:
1. a **written** notice;
2. from the **agency head or designee**;
3. stating the **reasons**, which must fall within §7922.535(c);
4. giving the **date a determination will be dispatched**;
5. that date being **no more than 14 days** out.

§7922.535(c) unusual circumstances, by paragraph opening:
1. the need to search and collect from separate facilities;
2. voluminous records;
3. consultation with another agency;
4. "The need to compile data, to write programming language or a computer program, or to construct a computer report to extract data." (verbatim);
5. agency inability to access its systems (cyberattack);
6. search during a declared state of emergency.

The section was last amended by Stats. 2025, ch. 34 (AB 370), eff. 2026-01-01. Inferred: that amendment added (5)–(6).

City practice (AR 95.21 §5.4):
- The 10 days are "calendar days," rolling to the next business day if day 10 falls on a weekend or City holiday.
- The 14-day extension "may only be asserted once."
- AR 95.21 lists only location, volume, and multiple agencies as unusual circumstances. The statutory data-compilation ground (c)(4) is absent from the City's AR, though the statute still controls.

On counting days, the CPRA doesn't say "calendar." The AR does. CCP §12a applies to Gov. Code deadlines (DeLeon v. BART (1983) 33 Cal.3d 456).

The determination is **not** the production deadline. Records must then be produced "promptly" (AR §5.4.3). Use the stated "estimated date and time" as the enforceable commitment.

**§7922.570** (verbatim, (a)–(b)): an agency holding a record in electronic format "shall make that information available in an electronic format when requested," "(1) ... in any electronic format in which it holds the information" and "(2) ... in the format requested if the requested format is one that the agency has used to create copies for its own use or for provision to other agencies."

**§7922.575(b)**: the requester bears programming/extraction cost only when "(1) ... the record is one that is produced only at otherwise regularly scheduled intervals" or "(2) The request would require data compilation, extraction, or programming to produce the record."

**§7923.610 / §7923.615** require police agencies to disclose arrest information and, for complaints and requests for assistance, "(A) The time, date, and location of occurrence. (B) The time and date of the report. ... (D) The factual circumstances surrounding the crime or incident." This is the basis for CAD-derived fields. §7923.600 (investigation records) is the exemption SDPD has used to withhold NPD rosters and officer identifiers.

### Submission routes

- **City of San Diego** (all departments: SDPD, ESD, HSSD): `https://sandiego.nextrequest.com/` (AR 95.21 §5.1; email fallback `publicrecords@sandiego.gov`). Requests are routed to departments by the PRA Program, but naming the department speeds assignment.
- **San Diego Housing Commission** (separate agency): its own portal at `https://sandiegohousingcommission.nextrequest.com/` ([sdhc.org/about-us/public-records-request/](https://sdhc.org/about-us/public-records-request/)). SDHC states the same 10-day / 14-day-extension framework.
  - Its appeal route: respond "within 10 business days" of SDHC's decision.
  - Media contact: mediarequests@sdhc.org.

### Common clauses (append to every request)

> This request is made under the California Public Records Act, Gov. Code §7920.000 et seq. I am requesting existing records only; I am not asking the agency to create analysis, summaries, or answer questions. Under §7922.570, please produce electronic records in the native or electronic format in which they are held (e.g., .xlsx/.csv export), not as PDF or image. If a record exists in more than one format, please produce the sortable electronic format. If any portion is exempt, please release every reasonably segregable portion after redaction (§7922.525(b)), identify the express exemption or balancing ground relied on for each redaction or withholding (§7922.000), and state any denial in writing (§7922.540(a)). If the agency contends that producing an export requires programming or extraction under §7922.575(b), please provide a written cost estimate before incurring it; I pre-authorize up to $[X]. Please make the §7922.535(a) determination within 10 days of receipt; any extension must be in writing, state a §7922.535(c) reason, and give the determination date. If no record matches a field I list, please produce the records that do exist and tell me which fields are not maintained, rather than closing the request as "no responsive records." Under §7922.600, if this request does not reasonably describe an identifiable record, please contact me to help focus it.

Every section cited in this clause was text-checked on leginfo on 2026-10-03:
- §7922.000 (the withholding agency must justify it);
- §7922.525(b) ("Any reasonably segregable portion of a record shall be available ... after deletion of the portions that are exempted by law"; amended Stats. 2024 ch. 80);
- §7922.540(a) ("A response ... that includes a determination that the request is denied, in whole or in part, shall be in writing.");
- §7922.600(a) (assist the requester: identify responsive records, "Describe the information technology and physical location in which the records exist," suggest ways around a practical basis for denial).

§7923.610 lists mandatory arrest disclosures: name, occupation, physical description, time/date of arrest and booking, location of arrest, factual circumstances, and further items through (i). It was checked only in summary.

---

### B1. SDPD, 63.0404 enforcement, Aug 2025 to present (gap after A1)
**To:** City of San Diego, Police Department (via sandiego.nextrequest.com)

> Please produce the SDPD Arrestee Log export, in the same .xlsx format released in PRA 25-6107 ("PRA_25-6107_Arrestee Log_01012021-07112025_Anonymized_SDUT_Taketa_SDPD.xlsx") and PRA 25-7564, for all arrests and citations (including cite-and-release) charging San Diego Municipal Code §63.0404, §63.0403, §54.0110, or Penal Code §647(e), with activity dates from **July 12, 2025 through October 3, 2026**. Please include the same fields as the 25-6107 release, plus the arrest/citation location (block-level is acceptable) as in the 25-6107 "Arrestee Log with Location" file, and the assigned division/command of the arresting or citing officer if that field exists in the export. Juvenile records may be withheld under W&I §827 as in 25-6107.
>
> Separately, please produce the existing export of SDPD field interviews (FI) recorded for SDMC §63.0404, §63.0403, §54.0110, or PC §647(e) contacts for the same period. This is the record type SDPD supplied from ARJIS to the Office of the City Auditor for report OCA-26-07, Exhibit 13. Include date, location (block-level), violation code, and outcome, as maintained. Subject names may be redacted.

Why this avoids "no records": it names a record SDPD has already produced twice, in the exact file format. Division is conditional, so its absence can't sink the request.

### B2. SDPD, NPD activity and deployment
**To:** Police Department

> Please produce the Neighborhood Policing Division "Recap for all squads" reports (the record released in PRA 22-5840 as "SDPD Recap for all squads 040119 to 103122.pdf") for **November 1, 2022 through October 3, 2026**, in the electronic format in which they are maintained (spreadsheet or native file if held; PDF otherwise). Officer names may be redacted; I do not request personnel rosters.
>
> Separately, please produce the Neighborhood Policing Division overtime report in the same form released in PRA 22-4629 ("NPD OT FY21 P6 - FY23 P2"), covering **FY23 Pay Period 3 through the most recent completed pay period as of October 3, 2026**.

Don't ask for an NPD roster or assignment list: 25-6154 was withheld under §7923.600/§7922.000.

Risk: the recap report may have been discontinued. The "tell me which fields are not maintained" clause keeps a successor report in scope. Optionally add: "or any successor periodic NPD squad activity report."

### B3. ESD, per-posting notice log and abatements after 2026-01-28
**To:** Environmental Services Department

> (1) Please produce the export from the system ESD uses to schedule and track encampment notices and abatements (the data underlying the monthly "Clean SD Summary" reports, e.g. PRA 26-4237) listing each **Notice of Clean-Up, notice of intent to abate, or other encampment notice posted by or for ESD**, from **July 31, 2023 through October 3, 2026**, with, as maintained: posting date and time, location/address or coordinates, notice type or timeframe (e.g., 3-hour, 24-hour, 72-hour), whether the notice was reposted, and the date of any resulting abatement. Native spreadsheet format. Also produce the existing log or index of the photographs/video recorded under SDMC §63.0406(c) for each abatement (date, location, file count). I do not request the images themselves.
>
> (2) Please produce an updated version of the record released in PRA 26-772 ("Copy of Abatements Completed 7.31.23 to 1.28.26.xlsx") covering **January 29, 2026 through October 3, 2026**, with the same columns.
>
> (3) Please produce the monthly Clean SD Summary reports for **May 2026 through September 2026**.

Part (1) is the novel item. Part (2) re-runs an export already produced in 10 days. If ESD says per-posting data isn't kept, items (2)–(3) still return records.

The system name is not verified; don't name one. Get it from the record via §7922.600 assistance, or from "FINAL Encampment Abatement Procedures 8.8.23.pdf" (25-360).

### B4. ESD, impounded property in native format
**To:** Environmental Services Department

> Please produce the Abatement Impound Log (the record released as PDF in PRA 24-4229, "Abatement Impound Logs 10.1.23 to Current", and in PRA 25-7303) and the Impounded Items Report (released in PRA 25-360) covering **October 1, 2023 through October 3, 2026**, **in the native electronic format in which they are maintained** (§7922.570(b)(1)), e.g. the .xlsx/.csv from which the PDFs were generated. Fields as maintained, including date impounded, location, item description, storage location, and disposition (retrieved by owner / disposed / date). Names and other personal identifying information of property owners may be redacted, as in 25-7303.
>
> Also produce any intake or retrieval log maintained by the facility where ESD stores impounded property (or by its contractor operating it, to the extent the City holds a copy) for the same period.

Optional companion request **to Police:** the property-room intake log entries for personal property impounded from persons arrested during encampment abatements under SDMC §63.0406(b)(3), for the same period. This is the SDPD-side impound stream; no prior request for it was found.

### B5. Shelter bed availability: send to SDHC, with a City backstop
**To:** San Diego Housing Commission (sandiegohousingcommission.nextrequest.com)

> Please produce existing records of daily (or most frequent available) bed inventory, occupancy, and/or available-bed counts for shelters administered by SDHC on behalf of the City of San Diego through its Memoranda of Understanding (e.g., 16th & Newton, 17th & Imperial, Single Adults Interim Shelter Program, Rachel's Promise, Veterans Shelter, and others listed on SDHC's City Homeless Shelters and Services page), including records generated by or for SDHC's Coordinated Shelter Intake, for **October 1, 2024 through October 3, 2026**. Acceptable records include: data exports or daily bed-availability reports used by Coordinated Shelter Intake; monthly occupancy/utilization reports submitted by shelter operators to SDHC under their contracts; and the data underlying SDHC's Homeless Programs Reporting Dashboards. Native spreadsheet format where held.

**City backstop. To:** Homelessness Strategies & Solutions Department (City portal)

> Please produce copies, in the City's possession, of any shelter occupancy or bed-availability reports received from SDHC or shelter operators (including reports attached to MOU/contract compliance submissions and reports to City Council committees), **October 1, 2024 through October 3, 2026**. If the City does not possess these, please identify the agency that does (§7922.600 / AR 95.21 §5.3.6).

Three City requests (22-2329, 23-4022, 26-7360) were referred out. An SDHC request naming a non-SDHC program (26-145) got "no records." So name SDHC-administered shelters. HMIS bed data held only by the Regional Task Force on Homelessness is likely outside CPRA (RTFH is a nonprofit; not verified).

### B6. SDPD overtime by division and event
**To:** Police Department

> Please produce, for **July 1, 2023 through October 3, 2026**: (1) the Labor Detail – Overtime report (the report type released in PRA 22-3442 and 24-6535) summarized by division/command and pay period, in native .xlsx format; and (2) for each special event for which SDPD prepared a police-services cost estimate or invoiced cost recovery (records of the type released in PRA 25-274: "Signed Final ... Cost Estimate" and "Final Invoice"), the cost estimate and final invoice. If the City's timekeeping system records an overtime project/activity/event code on overtime entries, please produce the existing export of overtime hours and cost by that code and division for the same period; I am not requesting that new codes or categories be assigned.

### B7. CAD: unit-level fields not in open data
**To:** Police Department

> The City publishes calls for service at data.sandiego.gov (pd_calls_for_service_*_datasd.csv). Please produce, for **January 1, 2025 through October 3, 2026**, the existing CAD export fields not included in that dataset, keyed by the same incident number: time dispatched, time first unit on scene, time cleared, and the responding unit's division or unit designator (not officer name or ID). Native .csv/.xlsx.

Context: 22-1579 had officer identifiers withheld under §7923.600. Asking for unit/division designators and times (§7923.615(a)(2)(A)–(B)) is narrower.

Inferred risk: SDPD may treat a new export as §7922.575(b)(2) programming. The pre-authorized fee cap in the common clauses handles that.

### B8. Fee estimates
No separate request is needed. Use the pre-authorization clause.

If a cost estimate arrives, ask in writing which §7922.575(b) prong applies. An export the City already runs for open data or prior PRAs is neither "produced only at otherwise regularly scheduled intervals" nor new programming.

To audit City practice on fees, optionally ask Public Records Administration for "records showing each PRA request from January 1, 2024 through October 3, 2026 in which the City issued a cost estimate or invoice under §7922.575, as maintained in the NextRequest portal's export." This has not been tried by anyone (no hits).

---

## Sources
- City NextRequest API (requests, request_documents, timeline): `https://sandiego.nextrequest.com/client/...`, accessed 2026-10-03
- SDHC NextRequest API: `https://sandiegohousingcommission.nextrequest.com/client/...`, accessed 2026-10-03
- City PRA page: https://www.sandiego.gov/communications/public-records-requests
- AR 95.21 Issue 2 (eff. 2022-11-22): https://www.sandiego.gov/sites/default/files/ar9521.pdf (pp. 1–8 read)
- SDHC PRA page: https://sdhc.org/about-us/public-records-request/
- SDHC City shelters page: https://sdhc.org/homelessness-solutions/city-homeless-shelters-services/
- City Coordinated Intake page: https://www.sandiego.gov/homelessness-strategies-and-solutions/services/coordinated-intake-system
- Police calls for service open data: https://data.sandiego.gov/datasets/police-calls-for-service/
- Gov. Code §§7922.000, 7922.525, 7922.535, 7922.540, 7922.570, 7922.575, 7922.600, 7923.610, 7923.615 via leginfo.legislature.ca.gov (through r.jina.ai)
- SDMC Ch. 6 Art. 3 Div. 4: https://docs.sandiego.gov/municode/MuniCodeChapter06/Ch06Art03Division04.pdf (pp. 1–6 read)
- City Auditor OCA-26-07 highlights (April 2026): https://www.sandiego.gov/sites/default/files/2026-04/unsafe-camping-ordinance-highlights.pdf
