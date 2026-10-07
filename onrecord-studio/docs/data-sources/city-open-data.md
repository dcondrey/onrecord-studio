# City of San Diego public-data sources for OnRecord Studio investigations

Checked 2026-10-03. Every URL below was fetched in this session unless marked otherwise.
Catalog: data.sandiego.gov is a static catalog over CSVs on `seshat.datasd.org` (no Socrata/SoQL API).
Authoritative distribution list: `https://raw.githubusercontent.com/COSD-PANDA/data-inventory/master/data.json`
(DCAT; large, WebFetch truncates it, so per-dataset landing pages were used for detail).

## 1. SDMC 63.0404 (Unsafe Camping Ordinance) enforcement

**No public record-level dataset exists.** Verified via the City Auditor's own inventory:

City Auditor, *Performance Audit of the City's Response to Homeless Encampments Since the Unsafe
Camping Ordinance*, OCA-26-07, April 2026 (PDF fetched, 7.2 MB):
https://www.sandiego.gov/sites/default/files/2026-04/performance-audit-of-the-city-s-response-to-homeless-encampments-since-the-unsafe-camping-ordinance.pdf
Highlights (2 pp): https://www.sandiego.gov/sites/default/files/2026-04/unsafe-camping-ordinance-highlights.pdf
- Scope FY2022 to FY2025. Exhibit 13 (source: "data SDPD provided from the Automated Regional Justice
  Information System (ARJIS)") gives fiscal-year totals for the Ordinance, Encroachment and Illegal
  Lodging combined: field interviews / citations / arrests = FY22 4,492/1,112/86 (5,690 total);
  FY23 3,602/1,674/196 (5,472); FY24 3,400/1,150/171 (4,721); FY25 4,730/1,605/391 (6,726).
  Note: aggregate, not split by statute, not by date/location/division. FY22 and FY23 predate the
  ordinance (effective July 29, 2023, enforcement from July 31, 2023), so those bars can only be
  illegal lodging and encroachment. Statutes named in the audit (r.jina.ai text extraction):
  SDMC 63.0404, SDMC 54.0110 and 63.0102(c)(12) (encroachment / temporary structures), and
  Penal Code 647(e) (illegal lodging). These are the search strings for RIPA `resulttext`/`code`
  and NIBRS `code_section`.
- Content of the internal SDPD monthly report (audit text, r.jina.ai): "monthly reports to City
  Management on the demographics of those contacted by SDPD for violations of the Ordinance",
  including "the monthly number of people contacted in field interviews, cited, or arrested for
  violations of the Ordinance, separated by ethnicity and gender". Monthly, aggregate, not public.
- The audit does not state which Get It Done values it filtered on; it says SDPD and ESD "independently
  monitor the Get It Done encampment reports assigned to their departments".
- Ordinance text (fetched via r.jina.ai): https://docs.sandiego.gov/municode/MuniCodeChapter06/Ch06Art03Division04.pdf
  SDMC 63.0404, O-21674 N.S., effective July 29, 2023. Prohibited at all times: within two blocks of
  schools with posted signs; within two blocks of shelters with posted signs; open space, waterways
  and banks; transit hubs, trolley platforms or tracks with posted signs; parks where the City
  Manager determines substantial public-health risk. The "posted signs" condition makes the
  signage-location list (internal) legally load-bearing for any sensitive-zone analysis.
- Council/committee search (sandiego.gov-scoped WebSearch, 2026-10-03) surfaced no SDPD enforcement
  update deck with monthly or division counts; only the audit, the 2023 fact sheet, IBA 23-15 review
  of the proposed ordinance, and community planning minutes. Not an absence proof.
- Appendix C "Public Resources" (p.56) states verbatim that "The San Diego Police Department's SDPD
  monthly reports on its response to encampments is shared with the Mayor's Office and interested
  Council offices" and is listed under information "tracked internally", i.e. NOT published.
  Same for SDHC daily shelter availability and Fire-Rescue's homelessness-fire dashboard.
- Appendix B (pp.51-53) names the non-public internal datasets the auditors used. These are the
  precise CPRA targets: (a) ARJIS data for violations of the Ordinance, Illegal Lodging and
  Encroachment (from SDPD); (b) City Attorney quality-of-life case outcome data; (c) monthly
  "Clean SD" data (ESD); (d) SDPD refusal and placement data; (e) locations of posted Ordinance
  signage (Mayor's Office); (f) SDHC Coordinated Shelter Intake and shelter referral/placement data;
  (g) Safe Sleeping program data (Dreams for Change, DSDP). Auditors also rode along with SDPD's
  Neighborhood Policing Division and observed ESD notice postings.
- Partial public proxies: RIPA (section 6) has PerceivedUnhoused, stopInResponseToCFS, isschool, and
  offense `code`/`resulttext` per stop result; whether SDMC 63.0404 is coded distinctly there is
  unverified (requires local download). RIPA excludes field interviews that are not detentions.
- Search for an SDPD UCO dashboard (WebSearch 2026-10-03) found only press coverage and the audit;
  no SDPD-published dataset.
- SDPD NPD UCO page (fetched): https://www.sandiego.gov/police/services/neighborhood-policing-division/unsafe-camping
  States the sensitive-area rule: enforcement regardless of shelter availability "Within two blocks
  of K-12 schools", "Within two blocks of a shelter", "Along trolley tracks and transportation hubs",
  City parks (where a health/safety risk is determined), open space/waterways. Links ordinance text
  https://docs.sandiego.gov/municode/MuniCodeChapter06/Ch06Art03Division04.pdf (linked, not fetched).
  Links NO data, statistics, or reports.
- Second public proxy: NIBRS (section 2b) carries `code_section` per offense and category
  "Curfew/Loitering/Vagrancy Violations" (Group B), with division and lat/long. Only offenses that
  produced an approved SDPD case report appear; whether "63.0404" strings occur in `code_section`
  is UNVERIFIED (download required).

## 2. Police calls for service / CAD dispatch

Dataset: https://data.sandiego.gov/datasets/police-calls-for-service/
- Description: "Calls dispatched by the San Diego Police Department's communications dispatch center."
  Excludes sensitive incident types (domestic violence, child abuse, suicide, sex crimes, stalking).
- Update: daily. Landing page "last updated" Oct 3, 2026. Range: 2015 to 2026, one CSV per year.
- CSV: `https://seshat.datasd.org/police_calls_for_service/pd_calls_for_service_{YYYY}_datasd.csv` (2015..2026)
- Dictionary (fetched): https://seshat.datasd.org/police_calls_for_service/pd_calls_for_service_dictionary_datasd.csv
- Fields: incident_num, date_time (24h), day_of_week, address_number_primary (abstracted to block),
  address_dir/road/sfx_primary, address_*_intersecting, call_type, disposition, beat, priority.
- Lookups (fetched): call types https://seshat.datasd.org/police_calls_for_service/pd_cfs_calltypes_datasd.csv ;
  dispositions https://seshat.datasd.org/police_calls_for_service/pd_dispo_codes_datasd.csv ;
  beats https://seshat.datasd.org/gis_police_beats/pd_beat_codes_list_datasd.csv ;
  priority defs (PDF, listed in dictionary, not fetched) https://seshat.datasd.org/police_calls_for_service/pd_cfs_priority_defs_datasd.pdf
- Call-type lookup (~370 rows; substring probe via r.jina.ai, complete per reader) has NO
  homeless, transient, encamp, camp, lodg, muni, encroach, outreach or quality-of-life type, and no
  647(e) (only 647B prostitution, 647F drunk). Nearest: 602 TRESPASSING, 415 family, CW CHECK THE
  WELFARE, 5150/5150V.
- Officer-initiated activity IS in CAD: `1151 PED STOP/FIELD INTERVIEW`, `1149`/`1150` TRAFFIC STOP,
  `SELENF SELECTIVE ENFORCEMENT`. So for the "did enforcement follow a real call" test, a 1151 or
  SELENF row at a block near a school/shelter/trolley with no preceding citizen-generated call at
  that block is the observable pattern (inference; there is no explicit self-initiated flag).
- Priority definitions (PDF fetched, read from disk; dated 08/26/2019): six levels. Priority Zero
  (also called "Priority E") imminent threat to life; One serious crime in progress/threat to life;
  Two less serious, no threat to life; Three minor/not urgent; Four "when no higher priority calls
  are waiting" (found property, parking); Nine Telephone Report Unit.
- Disposition codes: A arrest, R report, K no report required, U unfounded, O other, X/W/CAN cancels,
  DUP/V duplicate, *HR variants = Crisis Response Team. No citation-vs-arrest-by-statute detail.
- Gaps: no lat/long (block-level address only, must geocode); no unit/officer/division-unit
  assignment, no dispatch/arrival/clear timestamps (single date_time), so response times cannot be
  computed; no 63.0404 flag; officer-initiated enforcement that never became a CAD call is absent.
  Linking a stop to a call: not possible by key; only by time+block proximity (inference).

## 2b. NIBRS crime offenses (case-report level, complements CAD)

https://data.sandiego.gov/datasets/police-nibrs/ ; quarterly; updated Oct 3, 2026; Jan 2020 to present.
- CSV: `https://seshat.datasd.org/police_nibrs/pd_nibrs_{YYYY}_datasd.csv` (2020..2026)
- Dictionary (fetched): https://seshat.datasd.org/police_nibrs/pd_nibrs_dictionary.csv
- Fields: case_number, occured_on (landing page spelling; dictionary says `occurred_on`; the CSV
  header decides), approved_on, code_section, group_type (A/B), ibr_category,
  ibr_offense (+description incl. "Curfew/Loitering/Vagrancy Violations", "Trespass of Real
  Property", "Disorderly Conduct"), pd_offense_category, beat, neighborhood, service_area,
  division_number (1 Northern ... 9 Northwestern), division, block_addr, zip, geocode_status/score,
  latitude, longitude (block-level geocode, EPSG 4326).
- Gaps: no separate time field in the dictionary (occurred_on is a string; whether it carries a time
  is unverified); no CAD incident_num link to calls for
  service; offense-based, not enforcement-contact-based.

## 3. Get It Done (311), encampment reports

https://data.sandiego.gov/datasets/get-it-done-reports/ ; daily; updated Oct 3, 2026; May 2016 to present.
- CSVs: open requests https://seshat.datasd.org/get_it_done_reports/get_it_done_requests_open_datasd.csv ;
  closed by year `https://seshat.datasd.org/get_it_done_reports/get_it_done_requests_closed_{YYYY}_datasd.csv`
  (2016..2026); per council district `.../get_it_done_requests_CD{1..9}_datasd.csv` (listed in DCAT).
- Dictionary (fetched): https://seshat.datasd.org/get_it_done_reports/get_it_done_requests_dictionary_datasd.csv
- Fields: service_request_id, service_request_parent_id (duplicate linkage), sap_notification_number,
  date_requested (datetime), case_age_days, case_record_type (responsible staff group), service_name,
  service_name_detail, date_closed ("date and time when the service request was last modified";
  = close/referral date for closed/referred), status, lat, lng, street_address, zipcode,
  council_district, comm_plan_code/name, park_name, case_origin, referred, iamfloc, floc,
  public_description (free text from reporter).
- Encampment reports are inside this file (no separate encampment dataset in the catalog). The City
  Auditor (OCA-26-07, Appendix C item 1) names this dataset as the public source for "the City's
  response rate to Get It Done encampment reports", and computed ESD avg days-to-close FY22 63,
  FY23 47, FY24 17, FY25 16. The exact `service_name` string for encampments was not confirmed
  (files 14 to 140 MB exceed WebFetch); filter locally on service_name/case_record_type.
- SDPD also works these reports: the audit reports SDPD averaged 10 days to respond to Get It Done
  encampment reports in FY24 vs 17 in FY23, and says each department monitors the reports "assigned
  to their departments". `case_record_type` is defined as the grouping "that corresponds to the City
  staff group responsible for responding", so it (or `referred`) is the likely SDPD-vs-ESD split.
  Not verified against rows. If it holds, this file is the only public, record-level, geocoded,
  timestamped trail of SDPD encampment response (report created to closed), though it records
  closure, not the enforcement outcome.
- Gaps: no abatement outcome, no notice-posting time, no link to the ESD work order beyond
  sap_notification_number; date_closed is "last modified", so reopen/modify can move it.
  Separate-dataset siblings (graffiti, illegal dumping, 72-hour parking, potholes) exist; none for
  encampments.

## 4. ESD encampment abatement, notices, storage

**No public dataset.** Fetched pages:
- https://www.sandiego.gov/homelessness-strategies-and-solutions/services/abatement : notice "must be
  posted a minimum of 24 hours before the clean-up will begin"; expedited abatements with 3 hours'
  notice in emergencies; impounded items "stored for 90 days"; retrieval by phone 858-694-7000.
  No data, log, dashboard or inventory linked; no last-updated date.
- OCA-26-07 Appendix B lists "Monthly Clean SD data provided by ESD" as internal data given to
  auditors, and auditors "Conducted observations [of] ESD abatement notice postings and
  abatements". Question 5 (pp.43-48) covers storage; recommendations: periodic review of the
  not-stored items list; impound notices in Spanish and other languages (ESD agreed).
- Proxy only: Get It Done encampment reports with date_closed (section 3); Operating Actuals by
  ESD funds center (section 5) for spend.
- CPRA targets: Clean SD monthly data; abatement schedule/notice log with posting timestamps;
  impound/storage intake and release log (90-day retention); notice templates.

## 5. SDPD overtime, staffing, response times

Public machine-readable:
- Operating Actuals https://data.sandiego.gov/datasets/operating-actuals/ ; "as needed"; last updated
  Jul 31, 2026. CSV https://seshat.datasd.org/operating_actuals/actuals_operating_datasd.csv ;
  dictionary (fetched) https://seshat.datasd.org/operating_actuals/operating_actuals_dictionary_datasd.csv
  Fields: amount, report_fy, fund_type, fund_number, dept_name, funds_center_number, account,
  account_number. Annual totals per department x funds center x account, so overtime-type accounts
  by SDPD funds center (team/function/facility) are available; NOT by event, date, beat or officer.
  Exact overtime account names unverified (file not opened).
- Budget reference tables (from DCAT): https://seshat.datasd.org/accounts_city_budget/budget_reference_accounts_datasd.csv ,
  .../departments_city_budget/budget_reference_depts_datasd.csv , .../funds_city_budget/budget_reference_funds_datasd.csv.
  An "Operating budgets" dataset is also listed in the catalog (slug operating-budget) for budget-vs-actual pairing.
- No payroll / employee compensation / overtime dataset exists in the portal catalog (full 122-item
  catalog list fetched 2026-10-03).
Reports (narrative, PDF):
- City Auditor OCA-24-08, *Performance Audit of SDPD Overtime*, Feb 2024 (fetched via r.jina.ai):
  https://www.sandiego.gov/sites/default/files/2024-02/24-08_performance_audit_sdpd_ot.pdf
  Overtime detail lives in SAP (City timekeeping) plus paper overtime authorization forms; SAP
  tracks overtime by command (incl. Neighborhood Policing), by activity code (Patrol Backfill,
  Extension of Shift, Holiday Premium, Court, Clean SD, etc.) and by officer. Budget vs actual:
  FY19 $24.6M/$31.9M, FY20 $36.9M/$44.8M, FY21 $38.1M/$37.2M, FY22 $30.7M/$40.9M, FY23 $40.2M/$50.8M.
  No public overtime dataset referenced. (Figures: single-pass r.jina.ai extraction.)
- IBA Report 26-09, *Analysis of the Police Department's Fiscal Year 2026 Budget*, issued Mar 27, 2026
  (fetched via r.jina.ai): https://www.sandiego.gov/sites/default/files/2026-03/26-09-analysis-of-the-police-department-s-fiscal-year-2026-budget.pdf
  FY26 overtime budget $45.3M (-2.4% vs FY25 $46.4M); budgeted overtime by category incl. Patrol
  $9.5M, Neighborhood Policing $3.28M, Clean SD $2.39M, Special Events $5.15M; sworn FTE 2,031.00;
  Attachment A has vacancies by command. Budgeted, not actuals; no response times.
  (Single-pass extraction; verify figures in the PDF before publishing.)
- FY2026 Adopted Budget, Police volume (fetched via r.jina.ai):
  https://www.sandiego.gov/sites/default/files/2025-08/fy26ab_v2police.pdf
  Citywide KPIs only: avg time from call entry to first unit on scene, Priority 0 FY25 6.8 min
  (goal 7.0); Priority 1 34.9 (target 14.0); Priority 2 115.4 (27.0); Priority 3 166.6 (80.0).
  Not by division or beat. (Single-pass extraction; verify in the PDF.)
- SDPD 911 Monthly Reports https://www.sandiego.gov/police/data-transparency/911-monthly-reports
  (fetched): PDFs, Apr/May 2016 through Aug/Sep 2026; e.g.
  https://www.sandiego.gov/sites/default/files/2026-10/911rpts-260930.pdf (fetched via r.jina.ai):
  call-taking only (911 call counts, avg answer time 4.02 s, talk time, answer-time distribution,
  busiest day/hour). Citywide; no dispatch/arrival times, no division.
- SDPD Data & Transparency hub https://www.sandiego.gov/police/data-transparency (fetched): links
  911 reports, CPE report (stops/force 2016 to Sep 2020), crime statistics, force analysis, open data,
  Dispatch Online. No staffing, overtime, response-time-by-area, or encampment product.
- City Auditor OCA-21-004, *Performance Audit of SDPD's Data*, Sep 2020 (fetched via r.jina.ai):
  https://www.sandiego.gov/sites/default/files/21-004_sdpd_data.pdf  Relevant caveat: SDPD guidance
  told officers to report a single RIPA stop outcome although law requires all; ~6.8% of RIPA reports
  had multiple results; additional offenses often only in narratives.
Live view: SDPD Dispatch Online https://webapps.sandiego.gov/sdpdonline (fetched): live CAD feed,
"updated in five minute intervals", fields call date/time, call type, division, neighborhood, block
address; filter by division/neighborhood; no export; history depth unstated. Useful only for
forward-looking capture (scrape on a schedule), not for history.
Gaps: response times by division/beat, dispatch-to-arrival timestamps, overtime by event/date/
officer, and patrol deployment by beat are not public in any form found.

## 6. RIPA stop data (SDPD, AB 953)

Basic details: https://data.sandiego.gov/datasets/police-ripa-stops/
- Quarterly; landing page last updated Oct 2, 2026; July 2018 to present.
- CSV: https://seshat.datasd.org/police_ripa_stops/ripa_stops_datasd.csv (64 MB) and
  https://seshat.datasd.org/police_ripa_stops/ripa_stops_historic.csv (129 MB)
- Dictionary (fetched): https://seshat.datasd.org/police_ripa_stops/ripa_stops_dictionary_datasd.csv
- Fields of direct value: id, stopdate, stoptime, stopduration, **stopInResponseToCFS** (0/1),
  **stopmadeduringwelfarecheck** (0/1), stoptype (Vehicular/Bicycle/Pedestrian),
  officerAssignmentkey/assignment (RIPA assignment category, not SDPD division), intersection, block,
  landmark, street, **isschool**, **schoolName**, beat, beatName, PID, PerceivedAge/Gender,
  **PerceivedUnhoused** (0/1), IsStudent, InsideResidence.
Result of stop: https://data.sandiego.gov/datasets/police-ripa-stop-result/ (quarterly, updated Oct 3, 2026)
- CSV: https://seshat.datasd.org/police_ripa_stop_result/ripa_stop_result_datasd.csv and
  https://seshat.datasd.org/police_ripa_stop_result/ripa_stop_result_historic.csv
- Dictionary (fetched): https://seshat.datasd.org/police_ripa_stop_result/ripa_stop_result_dictionary_datasd.csv
  Fields: uid, id, pid, resultkey, result (warning/citation/custodial arrest/...), code (numeric
  offense code, float), resulttext ("specific violation if stop outcome is warning, citation or
  custodial arrest").
- Other RIPA tables (slugs on data.sandiego.gov/datasets/): police-ripa-actions-taken,
  police-ripa-stop-reason, police-ripa-race, police-ripa-gender, police-ripa-disability,
  police-ripa-non-force-actions, police-ripa-force-actions, police-ripa-search-basis,
  police-ripa-contraband-evid, police-ripa-prop-seize-type, police-ripa-prop-seize-basis.
  Pre-July-2018: police-vehicle-stops, police-vehicle-stops-search-details.
- Gaps: RIPA covers only "stops" (detentions/searches); consensual outreach contacts are presumably
  not reported (inference from the AB 953 stop definition, not checked against SDPD practice).
  Whether 63.0404 appears as its own `code`/`resulttext` value is UNVERIFIED (file too large for
  WebFetch; must be checked with a local download). RIPA `assignment` is the state category
  (e.g. patrol, gang enforcement, other), not "Neighborhood Policing Division".
- Statewide cross-check: CA DOJ OpenJustice https://openjustice.doj.ca.gov/data (fetched via
  r.jina.ai) lists annual RIPA ZIPs 2018..2024 (latest listed: 
  https://data-openjustice.doj.ca.gov/sites/default/files/dataset/2025-12/ripa-stop-data-2024.zip)
  and arrests 1980-2025 CSV (https://data-openjustice.doj.ca.gov/sites/default/files/dataset/2026-07/OnlineArrestData1980-2025.csv,
  county/agency aggregate). URLs taken from the fetched listing; the files themselves not downloaded.
  The City file is fresher (quarterly) and has beat/school fields; DOJ lags ~1 year.

## 7. Geographic layers

| layer | URL (landing) | direct files | source / freshness |
|---|---|---|---|
| Police beats | https://data.sandiego.gov/datasets/gis-police-beats/ | CSV https://seshat.datasd.org/gis_police_beats/pd_beats_datasd.csv ; codes https://seshat.datasd.org/gis_police_beats/pd_beat_codes_list_datasd.csv ; GeoJSON https://seshat.datasd.org/gis_police_beats/pd_beats_datasd.geojson (45 MB) ; TopoJSON .../pd_beats_datasd.topo.json ; SHP .../pd_beats_datasd.zip | weekly; updated Sep 29, 2026; beats roll up to service areas and divisions |
| Police divisions | https://data.sandiego.gov/datasets/gis-police-divisions/ | https://seshat.datasd.org/gis_police_divisions/pd_divisions_datasd.{csv,geojson,topo.json,zip} | weekly; updated Sep 29, 2026 |
| Council districts | https://data.sandiego.gov/datasets/gis-city-council-districts/ | https://geo.sandag.org/server/rest/directories/downloads/Council_Districts.{csv,geojson} ; _shapefile.zip ; _filegdb.zip | SanGIS, weekly; updated Aug 21, 2026; covers all districted cities in the county, filter to San Diego |
| Transit stops | https://data.sandiego.gov/datasets/gis-transit-stops/ | https://geo.sandag.org/server/rest/directories/downloads/Transit_Stops_GTFS.{csv,geojson} ; _shapefile.zip ; _filegdb.zip | SanGIS, MTS+NCTD GTFS; weekly; updated Jul 6, 2026. SANDAG also hosts Transit_Routes_GTFS, Transit_Stops_GTFS_Buffer, Transit_Priority_Areas services |
| Schools | https://opendata.sandag.org/Geographic-Information-Systems/Schools/ns7h-rni2 (metadata via https://opendata.sandag.org/api/views/ns7h-rni2.json) | ArcGIS FeatureServer https://geo.sandag.org/server/rest/services/Hosted/Schools/FeatureServer/0 (query returnCountOnly = 981 features, fetched) | CDE addresses geocoded by SanGIS; fields name, district, schoolid, priv, gsoffered, soctype, charter, opendate, cdscode, addr, city, zip. Metadata gives conflicting dates (dataUpdatedAt reported Jan 21, 2025; rowsUpdatedAt epoch 1772244726 = Feb 2026). Socrata `/resource/ns7h-rni2.json` returns empty objects (map asset); use the FeatureServer. Also School_Poly (parcels) service. No schools layer on data.sandiego.gov. |
| Shelters | none machine-readable | | SDHC list https://sdhc.org/homelessness-solutions/city-homeless-shelters-services/ (fetched): 14 City shelters with operator and bed counts, cross-streets for only two (16th & Newton, 17th & Imperial); no file, map, or date. No shelter service on geo.sandag.org (Hosted directory listing fetched). |
| Census tracts | geo.sandag.org Hosted/Census_Tracts_2020, Census_Tract_Tiger_2020 (directory listing) | | geometry only, no poverty |
| Poverty by tract | Census ACS 5-yr subject table S1701 | `https://api.census.gov/data/2024/acs/acs5/subject?get=NAME,S1701_C03_001E&for=tract:*&in=state:06%20county:073` | Fetch on 2026-10-03 returned "A valid key must be included with each data API request" (free key needed; not verified with a key). Keyless proxy: https://data.sandiego.gov/datasets/gis-cdbg-census-tracts/ (fetched; weekly, updated May 5, 2026): HUD block groups where at least 50% of residents are low/moderate income; binary eligibility, not a poverty rate; vintage not stated; download link not visible to the reader (JSON offered via the portal UI). |

Sensitive-zone buffers for 63.0404 ("two blocks") are not published; derive from schools/shelters/
transit stops. Ordinance signage locations are internal (OCA-26-07 App. B).

## 8. Capital spending and City property (Money pack)

Checked 2026-10-06: each landing page fetched, each CSV answered `HTTP 200` to a HEAD request and
was downloaded whole. Licence on every landing page: ODC-PDDL 1.0
(`https://opendefinition.org/licenses/odc-pddl/`). Each is one file, not split by year. All five
are in `ingestion::queue::city_datasets` and the builder's `san-diego-money` pack.

| CityDataset / pack table | Landing page | CSV (dictionary beside it) | Grain and columns | As seen 2026-10-06 |
|---|---|---|---|---|
| `CAPITAL_BUDGET` / `capital_budget` | https://data.sandiego.gov/datasets/cip-fy-budget/ | https://seshat.datasd.org/cip_fy_budget/budget_capital_fy_datasd.csv ; `capital_budget_dictionary_datasd.csv` | project, fund and account per fiscal year and budget cycle: amount, project_number, report_fy, budget_cycle, fund_type, fund_number, asset_owning_dept, project_name, account, account_number | 327,298 bytes, Last-Modified 2026-07-31. Adopted FY2011 to FY2025, proposed FY2024 and FY2025; no FY2026 rows although the file was rewritten in July 2026. |
| `CAPITAL_ACTUALS` / `capital_actuals` | https://data.sandiego.gov/datasets/cip-fy-actuals/ | https://seshat.datasd.org/cip_fy_actuals/actuals_capital_fy_datasd.csv ; `capital_actuals_fy_dictionary_datasd.csv` | parent and child project, fund and account per fiscal year: amount, project_number_parent, project_number_child, report_fy, fund_type, fund_number, asset_owning_dept, project_name, account, account_number | 66,116,235 bytes, 469,677 rows, Last-Modified 2026-07-31. FY2011 to FY2025. |
| `CITY_PROPERTIES` / `city_properties` | https://data.sandiego.gov/datasets/city-owned-properties/ | https://seshat.datasd.org/city_owned_properties/city_property_details_datasd.csv ; `city_property_details_dictionary_datasd.csv` | one acquisition file within a site: site_code, file_code, grantor, month/year acquired, purchase_fund, land/building/closing cost, site_name, managing_group, managing_dept, designated_use, acres, dedicated_park, use_restrictions, designating resolution and date | 882,528 bytes, Last-Modified 2026-10-06 08:00 GMT (rewritten daily). |
| `CITY_PARCELS` / `city_parcels` | https://data.sandiego.gov/datasets/city-owned-properties-parcels/ | https://seshat.datasd.org/city_owned_properties_parcels/city_property_parcels_datasd.csv ; `city_property_parcels_dictionary_datasd.csv` | one parcel of a site: site_code, `APN-8` | 92,153 bytes, Last-Modified 2026-10-06 08:01 GMT. |
| `CITY_LEASES` / `city_leases` | https://data.sandiego.gov/datasets/city-owned-properties-leases/ | https://seshat.datasd.org/city_owned_properties_leases/city_property_leases_datasd.csv ; `city_property_leases_dictionary_datasd.csv` ; rent codes `leases_rent_code_components_datasd.csv` (not fetched by the app) | one lease or permit on a site: site_code, lessee_name, Lessee Company, lessee_DBA, address_zip, lease_record_type, lease_description, lease_status, lease_location_name, nonprofit_lessee, date_effective, date_sched_termination, rent_code, cost_line_amt_USD | 154,427 bytes, Last-Modified 2026-10-06 08:01 GMT. |

`site_code` joins the three property files. The capital files name the department as
`asset_owning_dept`, not `dept_name`, so the operating budget and actuals checks never pick them up.
The capital budget is imported as the budget kind so `budget_cycle` stays a category; capital
actuals, like operating actuals, are imported as regional data. Importing the five files
downloaded 2026-10-06 through the app's analytical import rejected no row and left no cell
unconverted (2,193; 469,677; 4,095; 4,187; 810 rows). The M/D/YYYY date columns
(`date_effective`, `date_sched_termination`, `date_reso_ord`) are kept as recorded text, not dates.

Not added (blueprint F13 to F15, E10): none is on data.sandiego.gov or carries the PDDL. Goods and
Services contracts (`https://www.sandiego.gov/purchasing/bids-contracts/contracts2025`) and awarded
CIP construction contracts (`https://www.sandiego.gov/cip/reports/constructioncontracts`,
`/constructioncontractsfy24`) are HTML tables whose only linked files are per-contract PDFs; the
cumulative amounts paid to vendors are one PDF per fiscal year
(`https://www.sandiego.gov/sites/default/files/2025-07/cumulative-paid-fy25.pdf`). The relaunched
catalogue (123 datasets, fetched 2026-10-06) has no contracts or vendor-payment dataset.

## Needs with NO public source (CPRA required)

1. Record-level 63.0404 enforcement (citations, arrests, warnings, field interviews) by date, time,
   location, unit/division. Exists internally as ARJIS extracts and SDPD monthly encampment reports
   to the Mayor/Council (OCA-26-07 App. B and C). Public data has only FY totals (combined with
   illegal lodging and encroachment) in the audit.
2. Any NPD (Neighborhood Policing Division) identifier on stops, calls, or offenses. RIPA has only
   state assignment categories; CAD and NIBRS have geographic division, not unit. Partial exception
   to check: Get It Done encampment reports assigned to SDPD (likely via case_record_type) give
   record-level SDPD response timing, not enforcement outcome.
3. Field interviews (non-detention contacts): not in RIPA, CAD, or NIBRS.
4. ESD abatement logs, 24-hour notice postings (time/place), Clean SD monthly data, impound/storage
   intake and release records.
5. Ordinance signage locations (Mayor's Office) and an official sensitive-zone map.
6. Machine-readable shelter / safe-sleeping site locations and daily availability (SDHC daily
   availability is shared only with SDPD and partners).
7. CAD timestamps beyond call creation (dispatch, arrival, clear), unit IDs, officer-initiated flag;
   hence response times by division/beat.
8. Overtime by event/activity code, date, command or officer (SAP holds it); payroll/compensation.
9. City Attorney prosecution/case outcomes for 63.0404.

## Other portals checked
- County of San Diego: Socrata discovery API
  https://api.us.socrata.com/api/catalog/v1?domains=data.sandiegocounty.gov&q=homeless&limit=20 and
  https://data.sandiegocounty.gov/api/catalog/v1?q=homeless both returned resultSetSize 0 (2026-10-03).
  No county encampment, shelter-location, or City-enforcement dataset found.
- City Clerk: no machine-readable enforcement data found; Council/committee search above.
- data.sandiego.gov was relaunched in-house on 2026-06-04 (City press release, fetched via
  r.jina.ai: https://www.sandiego.gov/sites/default/files/2026-06/6-4-2026-city-of-san-diego-unveils-modern-open-data-portal-to-drive-transparency-efficiency-and-ai-innovation.pdf);
  no API announced; landing pages fetched today still point at seshat.datasd.org CSVs.

