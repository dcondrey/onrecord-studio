# Data packs: design and data dictionary

Draft 2026-10-04; build order and settled items 2026-10-06. Decision (owner): pre-acquire what publishers offer today, compile it into
versioned datasets, publish them on Hugging Face, and have the app import a pack and then fetch
only what was published after the pack's build date. Nothing is uploaded until the owner says so.
Source details and what was actually seen of each file are in `docs/data-sources/`.

## Rules

1. **Three layers per dataset.** The original file byte for byte with its SHA-256; a raw table
   holding every publisher column as text plus the whole row as JSON; a conformed table that adds
   the standard columns below. A column we later want is in the raw layer or can be recomputed
   from it, so no publisher column is ever chosen away.
2. **Additive schema.** A conformed column never changes meaning and is never removed. Each pack
   carries `schema_version`; a new column bumps the minor version.
3. **Recorded and derived are never the same column.** A value worked out by the pack builder
   (a council district from coordinates, a canonical department) sits beside the recorded value
   with a `_basis` column naming how it was derived. Absent is null, never zero or a default.
4. **Provenance is the builder's, stated as such.** A pack row was retrieved by the pack builder
   on the build date. The app labels it that way and can re-check any file against its publisher.
5. **Packs are signed.** The manifest lists every file with source URL, retrieval time, SHA-256,
   licence and row count, and is signed with a publisher key separate from the per-installation
   export key. The app refuses a pack whose signature or hashes do not verify.
6. **The app imports originals.** A pack ships original files and Parquet copies, not a DuckDB
   file, so it survives storage-format and schema changes (see the storage-version rule in
   `CLAUDE.md`).
7. **Mutable sources are snapshotted.** A publisher that overwrites a file (open reports, the
   short-term rental registry, a fixed-URL agenda, an editable map layer) gets one dated snapshot
   per build, appended, never replaced.

## Standard columns on every conformed table

| Group | Columns |
|---|---|
| Identity | `record_id` (publisher key, else a hash of the raw row), `publisher_record_id` |
| Provenance | `source_id`, `source_url`, `retrieved_at`, `source_sha256`, `source_row`, `pack_version` |
| Time | `occurred_at` as recorded with its offset, `occurred_on`, `calendar_year`, `fiscal_year` (City fiscal year, named by the year it ends), `time_basis` |
| Place | `address_recorded`, `latitude_recorded`, `longitude_recorded`, `council_district`, `council_district_basis`, `community_plan_area`, `census_tract_2020`, `zip`, `place_basis` |
| Parties | `agency_recorded`, `agency_id`, `department_recorded`, `department_id`, `organisation_recorded`, `organisation_id` |
| Money | `amount`, `amount_basis` (adopted, revised, actual, awarded, obligated, paid), `fund`, `account` |

## Reference tables

- `departments`: canonical id, every recorded spelling, valid-from and valid-to (renames and
  reorganisations; the homelessness department has no budget rows before fiscal year 2021).
- `funds` and `accounts`: from the City's budget reference files.
- `council_districts`: boundary polygons by era. Only the current lines are published; the
  pre-2021 lines must be obtained from an archive or by request before any record older than the
  redistricting is placed in a district.
- `geography`: census tract 2020, community plan area, ZIP, police beat, each to council district
  by area share, built from the SANDAG layers. No published crosswalk exists.
- `fiscal_calendar`, `court_holidays` (already in `analysis/court_calendar.rs`).
- `charge_sections`: section, short name, group (encampment, encroachment, lodging, urination),
  spellings seen (54.011 for 54.0110).
- `organisations`: vendors, lessees, shelter operators and grantees with aliases; fictitious
  business names link a trading name to a filing number, not to an owner.
- `meeting_bodies`: from `docs/data-sources/meeting-bodies.md`.

## Packs and their tables

Licence column: PD = public-domain dedicated or public record; ATTR = attribution required;
ASK = no licence stated, permission needed before publishing; COUNTS = publish aggregates only.

### Money
| Table | Source | Grain | Licence |
|---|---|---|---|
| `budget_adopted` | City operating budget | fund, department, account, fiscal year, cycle | PD |
| `budget_actuals` | City operating actuals | fund, department, account, fiscal year | PD |
| `homelessness_programme_funding` | Budget analyst report 26-19 and successors | programme by funding source | PD |
| `pay_by_position` | State Controller city pay file, 2017 to 2025 | position held, no names | PD |
| `settlement_resolutions` | Council resolutions index and PDFs | one settlement over 50,000 dollars | PD |
| `shelter_contracts` | Housing Commission staff reports | operator, programme, amount, term, beds | PD |
| `city_leases` | City property leases | lease | PD |
| `state_homelessness_grants` | HHAP fiscal snapshot, Encampment Resolution Fund status | grantee by round or site | PD |
| `federal_awards` | USAspending, HUD CoC award lists, single audits | award, audit, finding | PD |

### What residents report
| Table | Source | Grain | Licence |
|---|---|---|---|
| `service_reports` | Get It Done, all services, every closed year plus dated open snapshots | report | PD |
| `police_calls` | Calls for service 2015 on, with call-type and disposition lookups | incident | PD |

### Enforcement
| Table | Source | Grain | Licence |
|---|---|---|---|
| `police_stops` and companions | SDPD stop records | person stopped | PD |
| `arrests_regional` | ARJIS Group B arrests | arrest-offence, block address | ASK |
| `charge_counts` | Released SDPD arrest and citation exports | counts by section, month, beat, race | COUNTS |
| `abatement_notices`, `abatements_completed` | Released logs (25-5806, 26-772) | notice, abatement | PD |
| `parking_citations` | City parking citations | citation | PD |
| `police_practices_cases` | Commission on Police Practices | case | PD |

### Capacity and outcomes
| Table | Source | Grain | Licence |
|---|---|---|---|
| `shelter_inventory` | UCSD Homelessness Hub layer (from RTFH) | project by year | ASK |
| `housing_inventory_totals` | HUD per-CoC reports | project type by year | PD |
| `people_served` | Cal ICH system performance measures | measure by year | PD |
| `public_restrooms` | SDSU Project for Sanitation Justice layers; County site list | restroom, dated snapshot | ASK |
| `safe_sites` | Safe sleeping and safe parking sites | site, dated snapshot | ASK |
| `deaths_by_place` | County Medical Examiner cases | death | PD |
| `homeless_students` | State education department | school by year by dwelling type | PD |
| `housing_production` | State housing progress reports | project by year | PD |
| `short_term_rentals` | City licence registry (dated snapshots, without host contact fields) | licence | PD |

### Decisions
| Table | Source | Grain | Licence |
|---|---|---|---|
| `meetings`, `agenda_items`, `attachments` | Council and committee dockets; board pages | meeting, item, file, each with `version` and `posted_at` where a file carries one | PD |
| `resolutions` | Council resolutions and ordinances index | resolution | PD |
| `audit_recommendations` | City Auditor follow-up reports | recommendation with status | PD |

### Records access
| Table | Source | Grain | Licence |
|---|---|---|---|
| `requests`, `request_events`, `released_files` | NextRequest portals | request, event, file metadata (not the files) | PD |

### Influence
| Table | Source | Grain | Licence |
|---|---|---|---|
| `agency_reports`, `economic_interest_filings`, `campaign_filings`, `lobbyist_filings` | City disclosure portal | filing | held back: the portal's terms bar redistribution for commercial use |
| `business_names` | County fictitious business names | name on a filing | PD |

## Acceptance questions

A pack set is complete when each question is answered by a join over conformed tables, and the
answer names every table and file it read. A question that cannot be answered names its missing
column or table here before anything is published.

1. By fiscal year: adopted and actual spending for the homelessness department, Encampment
   reports filed, and year-round emergency shelter beds. (Built in the app as the capacity lead.)
2. By council district and year: shelter beds, Encampment reports, abatements completed, and
   police calls of type SLEEPER, each placed by the district lines in force that year.
   *Blocked on pre-2021 district boundaries.*
3. For each shelter contract: dollars per bed per year, and the operator's other City contracts
   and leases.
4. Hours from a posted notice to the abatement at the same place, set against the notice period
   the City's procedure states. *Blocked on the procedure document as a stored authority.*
5. Arrests and citations under encampment, encroachment, lodging and urination sections by
   month and beat, beside public restrooms open at that hour within a quarter mile.
6. Overtime paid by department and year against the adopted overtime line.
7. Settlements by department and year, and whether the claim's incident type recurs.
8. For each agenda item on these subjects: hours between the file's posting and the meeting, and
   what changed between versions.
9. Records requests on a subject by agency: how many closed with no responsive records, and how
   many of those were later contradicted by a release.
10. State grant dollars awarded, obligated and spent by grantee and round, beside the outcomes
    the grantee reported.
11. Deaths recorded at an encampment, on a sidewalk or street, by ZIP and month, beside shelter
    beds that month.
12. Vendors receiving a contract within a year of a behested payment or gift report naming them.
    *Uses the Influence pack, which is not published.*

## Settled (2026-10-06)

- **City open data licence.** Each City dataset page links ODC-PDDL 1.0
  (`https://opendefinition.org/licenses/odc-pddl/`). The City Open Data Policy (effective
  2015-01-01, `https://www.sandiego.gov/sites/default/files/open-data-policy.pdf`, SHA-256
  `71c27bfb…a3569`) places no restriction on lawful use (s. 3.5), allows commercial use (s. 2.7),
  and requires a disclaimer: informational only, no warranty, no City liability. A pack manifest
  carries the licence id, licence URL and that disclaimer; no attribution field is needed.
- **Publisher key.** Ed25519, signed and verified with `integrity::sign_package` /
  `verify_package` over canonical manifest bytes. The private seed is kept in the macOS Keychain
  (service `com.onrecord.pack-publisher`), never in the repository; the public key is pinned in the
  app as `PACK_PUBLIC_KEYS`, separate from the per-installation export key and the entitlement keys.
- **Hugging Face.** Account `davidcondrey`, organisation `Writerslogic`; packs publish to
  `Writerslogic/onrecord-<pack>`. Creating a repository or uploading waits for the owner.
- **Pack origin.** A snapshot imported from a pack keeps the identity the analytical import gives
  it; the pack is recorded beside it (pack, version, build time, file SHA-256), so the same bytes
  fetched by the user and shipped in a pack are one snapshot with two recorded origins, and the
  builder's retrieval is labelled as the builder's.

## Build order

1. **First slice: Money and What residents report, City open data only.** Tables
   `budget_adopted`, `budget_actuals`, `service_reports` (closed years and one dated open
   snapshot) and `police_calls` (by year). Acceptance: question 1 without shelter beds
   (`shelter_inventory` is ASK), answered by a query that names every table and file it read.
2. **Builder.** In the app library behind the `pack-builder` Cargo feature, with a thin binary,
   so the app's feature set and binary are unchanged. It takes URLs, years and filters from
   `ingestion::queue::city_datasets` (one source list), writes originals with SHA-256, records the
   publisher's freshness markers (year, `Last-Modified`, `ETag`), writes Parquet copies (DuckDB's
   `parquet` feature is enabled only by `pack-builder`), and signs the manifest.
3. **Conformed layer.** One module generates the standard columns from an imported table; the
   builder uses it for the Parquet copies and the app after importing originals. First slice:
   identity, provenance and time groups (`fiscal_year` for question 1). Place waits for the
   pre-2021 district lines.
4. **App import.** Verify the signature and every file hash before writing anything; then import
   originals through the analytical import, one table and year at a time, chosen by the user
   (`MAX_STREAMED_CELLS` is per import). Tamper tests: one byte changed in a file, and in the
   manifest, each refused.
5. **Later.** Fetch only what was published after a pack's build date, using the recorded
   freshness markers; the remaining PD packs; ASK tables once permission is in writing.

## From the research blueprint (owner's file, 2026-10-06)

`San_Diego_Research_Blueprint.xlsx` (10 investigations, 13 proposed datasets, 56 sources) adds
City open-data sources that fit the Money pack under the same licence, and two dataset designs the
rules above already need:

- **Money pack additions (City open data, PDDL):** Capital Improvements Program budget and actuals
  by fiscal year (F03, F04); Goods and Services contracts (F13); awarded CIP construction contracts
  (F14, E10); cumulative amounts paid to vendors by fiscal year (F15); City-owned properties,
  parcels and leases (R02 to R04). Next after the first slice; each needs an entry in
  `ingestion::queue::city_datasets` first, since the builder reads only that list.
- **Other City open data it names:** development permits (H01), completed paving and street
  segments (E07, E08), stormwater conveyance GIS (E02).
- **D13, source snapshots and change history,** is rule 7 plus the manifest's freshness markers.
- **D08, verified entities and record crosswalks,** is the `organisations` reference table: one
  evidenced identity or identifier relation over time (vendor ids, EINs, APNs, CIP numbers).
- Its investigations I06 (encampment interventions beyond report closure) and I08/I09 (contract
  lifecycle, payment timing) use the Enforcement and Money packs; its records that are documents
  (DS-450 forms, Housing Commission packets, audit reports) are collected sources, not pack tables.

## Open

- Written permission, or a stated licence, from the UCSD Homelessness Hub, the SDSU sanitation
  project and the Regional Data Library.
- Pre-2021 council district boundaries.
- Owner's go-ahead to create the Hugging Face repositories and upload.
