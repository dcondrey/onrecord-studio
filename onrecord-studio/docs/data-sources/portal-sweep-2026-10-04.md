# Portal sweep: what to take, from where, under what licence

Checked 2026-10-04 through a summarising fetcher; files over 10 MB were located, not opened.

## New datasets worth adding

| Dataset | Where | Grain, years, columns | Licence | Answers |
|---|---|---|---|---|
| Short-term rental licences (City) | `seshat.datasd.org/stro_licenses/stro_licenses_datasd.csv` (+ tier definitions, dictionary) | one active licence, current snapshot only: license_id, address, tier, community_planning_area, date_expiration, rtax_no, lon/lat, council_district, host and contact names and phones | PDDL | licensed rentals by district and tier. No history is published, so snapshot it. Holds personal data |
| Inside Airbnb San Diego | `data.insideairbnb.com/united-states/ca/san-diego/<date>/visualisations/listings.csv` | one listing with `license`, host_id, host listing count | attribution required; no formal licence | listings without a licence, multi-listing hosts |
| Homeless student enrollment (CDE) | `www3.cde.ca.gov/demo-downloads/homeless/hse<YYYY>.txt`, 2019-20 to 2024-25 | state, county, district and school, by dwelling type | not stated | student homelessness by school |
| Rental unit business tax accounts (City) | `seshat.datasd.org/rtax_accounts/rtax_accounts_datasd.csv` | account, parcel, address | PDDL | rental stock by parcel |
| Development approvals (City) | `seshat.datasd.org/development_permits/approvals_{created,issued,closed}_<YYYY>_datasd.csv`, 2024 on | one approval: project, parcel, coordinates, type, status, valuation, net dwelling units | PDDL | housing approved by place |
| County permits and environmental health | civicdata.com County organisation, CKAN datastore, 39 datasets | building permits, planning applications, inspections, housing facilities (unincorporated area, inferred) | public domain | |
| Live Well San Diego indicators | `data.sandiegocounty.gov/resource/wsyp-5xpf.json` | 74 areas by 14 years (2012 to 2025): life expectancy, income, housing-cost share, food insecurity, crime rates. No homelessness or poverty-rate column | public domain | context by subregional area |
| Fair Market Rents (HUD) | `huduser.gov/portal/datasets/fmr/fmr<YYYY>/` workbooks, ZIP-level small-area files; companions: income limits, CHAS, Picture of Subsidized Households, LIHTC, USPS crosswalk | metro and ZIP by year, 2001 to 2026 | federal, not stated | rent benchmarks by ZIP |
| Traffic collisions (City) | `seshat.datasd.org/traffic_collisions/pd_collisions_datasd.csv`; per person `.../traffic_collision_details/pd_collisions_details_datasd.csv` | 2015 on: date, beat, address, violation, injured, killed, role | PDDL | pedestrian deaths and injuries by place |
| Street sweeping schedule (City) | `seshat.datasd.org/gis_street_sweeping/street_sweeping_datasd.csv` | block: posted, schedule, council_district | PDDL | where posted sweeping exposes parked vehicles to citation (inferred use) |
| Hate crimes (City) | `seshat.datasd.org/hate_crimes/hate_crimes_datasd.csv` | 2025 on; a 2016 to 2025 set exists | PDDL | |

Stale City sets: campaign finance (2007 to 2021), code enforcement (2015 to 2018; current data is in
OpenDSD), special events (to 2021). Not in the City's catalogue at all: restrooms or park amenities,
streetlights, sidewalks, library or recreation-centre hours, lobbyist data, pre-2021 council
districts. The City's dataset index: `raw.githubusercontent.com/COSD-PANDA/data-inventory/refs/heads/master/data.json`.

## Verdict per portal

Origin: data.sandiego.gov (122 datasets); geo.sandag.org hosted layers (no account; one boundary
vintage only; `Parcels_South` has no owner name); opendata.sandag.org (also SWITRS collisions
`uzct-sb5t`, crime rates `ydfx-c888`, clearance rates `ivgr-tihw`); data.sandiegocounty.gov;
civicdata.com County set; Inside Airbnb; HUD USER; CDE.
Copy or pointer, take from the origin instead: catalog.data.gov (1,057 hits, mostly federal
science; API 404), data.ca.gov city page, opendatasandiego.com, sandag.org pages,
rdw.sandag.org (account required).
Link lists: UCSD Data Nexus, UCSD library guide.
Retired or dead: sdgis-sandag.opendata.arcgis.com, kb.sandiegodata.org, DataPlanet.
Dashboards only: data.sdcoe.net. Not relevant: CAIDA, CDIP, water-quality repository, SDSU HDMA.
Reference for query patterns (MIT code): `sdgis-cli` on PyPI, `codeforanchorage/sandiego-gis-mcp`.

## Existing derivatives that could be extended

San Diego Regional Data Library (Eric Busboom, Civic Knowledge): WordPress catalogue
`data.sandiegodata.org/wp-json/wp/v2/search?search=<kw>&per_page=50`; files at
`library.metatab.org/<pkg>-<ver>/data/*.csv`; frozen 2019 to 2022; no licence stated on any
package (ask eric@sandiegodata.org before reuse).
- Downtown San Diego Homeless Sleepers v2.1.1: point-level digitisation of the Downtown
  Partnership's hand-drawn maps 2014 to mid-2018, monthly totals from 2012. Unique.
- San Diego Homelessness Contract Annotations v1.1.1: annotated contracts collected by the County
  Taxpayers Association.
- ARJIS crime incidents with victim descriptions, 2016 to mid-2020, obtained by records request.
- Community, city and tract boundaries 2018: tract to city and community-plan link. No council
  district link; 2010 tracts (inferred).
- HIC facility geocoded addresses (superseded by the UCSD layer).

## Licences for redistribution

Public domain or dedicated: City open data (ODC-PDDL), County Socrata and civicdata sets. Inferred
public domain: HUD, CDE. Attribution required: Inside Airbnb. Unclear: Regional Data Library
packages, SANDAG Socrata sets, SANDAG and SanGIS layers taken directly, UCSD Homelessness Hub
layers. The City's terms page was read only in summary; read it in full before publishing a pack.

## Public restrooms: Project for Sanitation Justice (SDSU)

The best restroom inventory found. The site has no download link, but the map's layer is queryable:
`https://services1.arcgis.com/SIYkiqjmENweC50g/arcgis/rest/services/V2_Public_Restrooms_in_San_Diego_093022/FeatureServer/0`
(802 rows countywide, one per restroom, 2,000 records per call; item last modified about September
2024). Fields: ID_4Digit, Name, Latitude, Longitude, Address, Advertised_Hours, Confirmed_Hours_,
Showers, Menstrual_Products, ADA_Accessible, Baby_Changing, Gender_Neutral, Restroom_Open_Status,
Restroom_Assessed, Category, Operated_By, Notes, assess_date. Most coded fields are integers with
no published code list; no cleanliness column. Seen: 1321, Pacific Beach Law Street, 4861 Ocean
Blvd, advertised 24/7, operated by City of San Diego, assessed June 2022.

Transit sub-study layers on the same host, with text values: `MTS_Restrooms__Open_Access`,
`MTS_Restrooms_Closed`, `MTS_Restrooms_Subjective_Access`, `MTS_Restrooms__Excluded` (facility
status, trolley station within a quarter mile, operator, advertised and confirmed hours).
Catalogue: `arcgis.com/sharing/rest/search?q=owner:mswayne_SDSUGeo&num=50&f=json` (43 items).

Method: Swayne, Calzo, Felner, Welsh-Carroll, PLoS ONE 18(7) e0288525 (2023), CC BY: 669 public
restrooms countywide at publication; 57 community planning areas averaged five, range 0 to 33;
of seven Pacific Beach restrooms assessed in 2022, posted hours were found at two.
Licence: none on the layers; the paper says the location data is freely available. Confirm with
the team before republishing. The layer is editable upstream, so snapshot it.
