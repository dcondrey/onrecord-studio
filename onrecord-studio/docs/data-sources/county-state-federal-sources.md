# County, regional, state and federal sources

Checked 2026-10-04 through a summarising fetcher. "Seen" means San Diego rows or values were read;
files listed as "not opened" have a confirmed URL only. No source needed a login.

Identifiers: City of San Diego UEI L4FACR9F6GK7, EIN 956000776, SDPD ORI CA0371100. County UEI
CZTNNM6XJVJ3, EIN 956000934, Sheriff ORI CA0370000. San Diego Housing Commission UEI GGW6JT2VNEA1,
EIN 953390896. Regional Task Force on the Homeless UEI FR4UK6EFRKU3, EIN 113723093, CoC CA-601.

## County and regional

| Source | Where | Grain and columns | Answers |
|---|---|---|---|
| ARJIS arrests, Group B (SANDAG) | `opendata.sandag.org/resource/7um2-24et.json` (Socrata; CSV at `.csv`) | one arrest-offence, 112,669 rows, 2021 to 2026-09-30, weekly: arrest_number, arrest_date, arrest_agency, offense_code, offense_description, block_address, zip, beat, census_tract. Counts: disorderly conduct 30,905, trespass 2,722, curfew/loitering/vagrancy 2,284. No statute section, no demographics, no housing status | who is arrested where for conduct offences |
| ARJIS incidents, Group A | `opendata.sandag.org/resource/7sps-5pd9.json` | 794,703 rows, victim age, sex, race | |
| Medical Examiner cases (County) | `data.sandiegocounty.gov/resource/jkvb-n4p7.json` | one death, 89,875 rows to 2026-07-31: death_date, age, gender, race, manner, cod_string, how_injury_occurred, opioid_related, event_place, event_place_type, event_zip, death_place_type, res_zip. No housing-status column; proxies are free-text `event_place` (2025: sidewalk 67, encampment 23, street 121, park 21) and `death_place_type` jail (142 total). 901 rows sealed | deaths on streets, at encampments and in jails |
| County budget and actuals | `data.sandiegocounty.gov/resource/32sf-9pw4.json` (revenue `paik-44xk`) | FY2025 only, 27,758 rows: department, program, account, adopted_budget, amended_budget, actual, encumbrance | County budget against spending, with a programme column |
| County Levine Act report | `data.sandiegocounty.gov/resource/vw4e-55mk.json`; procurement forecast `rrhp-9sw9` | matter, department, Board decision date, reported campaign contribution | contributions from parties with matters before the Board |
| Jail-interview aggregates (SANDAG) | `opendata.sandag.org/resource/yq95-cfh9.json`; `fe43-4kyb` | question, response, year, frequency, percentage | share of booked arrestees with no fixed residence |
| Geography layers (SANDAG) | `geo.sandag.org/server/rest/services/Hosted/<Name>/FeatureServer/0/query?where=1%3D1&outFields=*&f=json` | `Council_Districts`, `Census_Tracts_2020`, `CMTY_PLAN_SD`, `SDPD_Beats`, `Zipcodes`, `Parcels`, `Affordable_Housing_Inventory` | placing any record in a district, tract or plan area |
| RTFH monthly HMIS newsletters | `rtfhsd.org/wp-content/uploads/<YYYY>/<MM>/HMIS-Data-Newsletter-<Month>-<YYYY>.pdf`, monthly since 2022-12 | PDF: people housed against people homeless for the first time | inflow and outflow by month |
| Review board on Sheriff complaints (CLERB) | `sandiegocounty.gov/content/sdc/clerb/meetings/<YYYY>.html` | monthly PDFs: findings, workload report | sustained complaints and deaths reviewed |
| Sheriff transparency reports | `sdsheriff.gov/resources/...` | PDFs: stops 2018 to 2024, force 2016 to 2023, monthly jail population; in-custody deaths table is an image | |
| County Grand Jury | `sandiegocounty.gov/content/sdc/grandjury/reports/<YYYY-YYYY>.html` | PDFs with agency responses | |
| Downtown Partnership monthly unsheltered count | `downtownsandiego.org/wp-json/wp/v2/media?search=unsheltered` | PDFs; first page tabulates monthly totals back to 2012 (from an earlier check, not re-fetched) | |

Catalogue trap: `data.sandiegocounty.gov/api/catalog/v1` is federated and returns other counties'
datasets; use `api.us.socrata.com/api/catalog/v1?domains=data.sandiegocounty.gov&search_context=data.sandiegocounty.gov`.

Nothing published: District Attorney or City Attorney case outcomes; Superior Court data by charge
(index excludes minor offences, name search only, 403 to the fetcher); MTS citations and fare
enforcement; Caltrans District 11 or Port encampment records; 211 San Diego data (dashboard only);
County homelessness dashboard (Power BI, no export).

## State and federal

| Source | Where | Grain and columns | Answers |
|---|---|---|---|
| HHAP fiscal snapshot (HCD) | `hcd.ca.gov/sites/default/files/docs/grants-and-funding/calich/hhap-dashboard-fiscal-snapshot.xlsx` | one sheet per round; grantee: total award, obligated, expended, unexpended; filter Region `CA-601`. Seen (round 1): City 22,491,840.12, fully spent; County 9,996,652.14; CoC 10,790,528.04. Rounds 3 to 6 not seen | did each grantee spend its state homelessness money |
| Encampment Resolution Fund status (HCD) | `hcd.ca.gov/sites/default/files/docs/grants-and-funding/calich/erf-all-rounds-encampment-projections-and-status.xlsx` | grantee by site, self-reported, as of 2026-03-31. Seen: City I-15 Corridor resolved; County Riverbed Corridor and Plaza Bonita not resolved; CoC State Route 94 not resolved | which funded encampment sites were reported resolved |
| OpenJustice (CA DOJ) | index via `openjustice.doj.ca.gov/data` (JavaScript); files under `data-openjustice.doj.ca.gov/sites/default/files/dataset/<YYYY-MM>/` | stops by county per year 2018 to 2024 (with perceived housing status), use of force 2016 to 2025, deaths in custody 2005 to 2025, complaints against officers by agency, arrests and dispositions 1980 to 2025. File names change yearly; not opened | force, deaths and complaints by agency |
| Federal Audit Clearinghouse | `api.fac.gov/general?auditee_uei=eq.<UEI>&api_key=`; `/findings?report_id=eq.<id>` (free api.data.gov key) | one audit; findings. Seen: City 2024, 348,115,359 federal dollars spent, one finding. Before 2022 filter by EIN | single-audit findings and federal dollars |
| CourtListener | `courtlistener.com/api/rest/v4/search/?type=r&party_name="City of San Diego"&nature_of_suit=440&court=casd` | one federal docket; 292 hits; 15 with the Housing Commission as a party. Use the `party_name` parameter, not `q=` | civil-rights suits against the City |
| HUD CoC awards | `files.hudexchange.info/reports/published/CoC_AwardComp_CoC_CA-601-<YYYY>_CA_<YYYY>.pdf` | text PDF: recipient, project, type, amount. 2024: 44,053,677 across 60 projects | who receives federal CoC money |
| Housing production (HCD) | `data.ca.gov` resource `fe505d9b-8c36-42ba-ba30-08bc4f34e022`, filter `JURIS_NAME = SAN DIEGO` | one project per year, 15,014 City rows: address, coordinates, units entitled, permitted and completed by income band | housing built by income level and place |
| Hospital encounters, homeless patients (HCAI) | `data.chhs.ca.gov` dataset `hospital-encounters-for-homeless-patients` | facility by demographic: encounters, share | hospital burden by facility |
| State Controller city finances | `bythenumbers.sco.ca.gov/resource/ju3w-4gxp.json?entity_name=San Diego` | category by year 2003 to 2024. Seen: police current expenditures 622,377,651 in 2024; housing 2.6M (2021) to 38.5M (2024). No liability-claims category | spending by function over time |
| State grants portal | `data.ca.gov` grant-awards CSV per fiscal year | recipient, amount, abstract; thin coverage | |
| State Auditor recommendations | `auditor.ca.gov/reports/recommendations/<report>`; agency index `information.auditor.ca.gov/reports/agency/517` | HTML: recommendation and status | did the City act on state audits |
| USAspending | `POST api.usaspending.gov/api/v2/search/spending_by_award/` with `recipient_search_text: [<UEI>]` | not executed (POST only from the fetcher); listing numbers from recall need checking | federal awards by recipient and programme |
| Not opened, URL confirmed | HUD Picture of Subsidized Households (`huduser.gov/portal/datasets/pictures/files/PHA_2025_2020census.xlsx`), Treasury ARPA quarterly data, tax-credit project list (`treasurer.ca.gov/sites/default/files/ctcac/List of Projects_1.xlsx`) | | |

PDF or dashboard only: Homekey awards, Behavioral Health Bridge Housing, Caltrans encampment
funding report (16,560 removals July 2023 to September 2025 per a search snippet, unverified).
Blocked to the fetcher: State Controller pay site, Judicial Council court statistics. Census ACS
needs a free key. No published crosswalk from census tract to council district; derive it from
the district boundaries.
