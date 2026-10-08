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
| Identity | `record_id` (publisher key, numbered when the file repeats it, else a hash of the raw row), `publisher_record_id` |
| Provenance | `source_id`, `source_url`, `retrieved_at`, `source_sha256`, `source_row`, `pack_version` |
| Time | `occurred_at` as recorded with its offset, `occurred_on`, `calendar_year`, `fiscal_year` (City fiscal year, named by the year it ends), `time_basis` |
| Place | `address_recorded`, `latitude_recorded`, `longitude_recorded`, `council_district`, `council_district_basis`, `community_plan_area`, `census_tract_2020`, `zip`, `place_basis` |
| Parties | `agency_recorded`, `agency_id`, `department_recorded`, `department_id`, `organisation_recorded`, `organisation_id` |
| Money | `amount`, `amount_basis` (adopted, revised, actual, awarded, obligated, paid), `fund`, `account` |

A publisher column whose name is a standard column's (Cal ICH's `calendar_year`, the State
Controller's `Fiscal Year`) keeps its values as `<name>_as_published`, with `_as_published`
appended again until the name is free. The standard column keeps its meaning: it is derived only
from the table's time column and is never filled from the publisher's column of that name.

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
| `capital_budget` | City capital improvements budget | project, fund, account, fiscal year, cycle | PD |
| `capital_actuals` | City capital improvements actuals | parent and child project, fund, account, fiscal year | PD |
| `city_properties` | City-owned properties | acquisition file within a site, dated snapshot | PD |
| `city_parcels` | City-owned property parcels | parcel (APN) of a site, dated snapshot | PD |
| `homelessness_programme_funding` | Budget analyst report 26-19 and successors | programme by funding source | PD |
| `pay_by_position` | State Controller city pay file, 2017 to 2025 | position held, no names | PD |
| `settlement_resolutions` | Council resolutions index and PDFs | one settlement over 50,000 dollars | PD |
| `shelter_contracts` | Housing Commission staff reports | operator, programme, amount, term, beds | PD |
| `city_leases` | City-owned property leases | lease or permit on a site, dated snapshot | PD |
| `budget_accounts`, `budget_departments`, `budget_funds` | City budget reference files | account, funds center, fund | PD |
| `state_homelessness_grants` | HHAP fiscal snapshot, Encampment Resolution Fund status | grantee by round or site | PD |
| `federal_awards` | USAspending, HUD CoC award lists, single audits | award, audit, finding | PD |

### What residents report
| Table | Source | Grain | Licence |
|---|---|---|---|
| `service_reports` | Get It Done, all services, every closed year plus dated open snapshots | report | PD |
| `police_calls` | Calls for service 2015 on, with call-type and disposition lookups | incident | PD |
| `call_types`, `call_dispositions`, `police_beats` | The calls' lookup files | code | PD |
| `fire_ems_incidents` | Fire-Rescue responses 2007 on | response | PD |

### Enforcement
| Table | Source | Grain | Licence |
|---|---|---|---|
| `police_stops` and companions | SDPD stop records | person stopped | PD |
| `police_offenses` | SDPD NIBRS offences 2020 on | offence on a case | PD |
| `traffic_collisions`, `traffic_collision_people` | SDPD collision reports 2015 on | report; person in a report | PD |
| `hate_crimes` | SDPD hate crimes | case | PD |
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
| `approvals_created`, `approvals_issued`, `approvals_closed` | City development approvals 2024 on, by the year of each stage | approval | PD |

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
  `Writerslogic/onrecord-<pack>` (public datasets; all four packs' manifests resolved on
  2026-10-07: money, reports, enforcement and capacity, each checked file by file against its
  manifest after upload). The app fetches a pack's manifest from
  `https://huggingface.co/datasets/Writerslogic/onrecord-<pack>/resolve/main/manifest.json`,
  verifies it, and fetches the chosen files from the commit that manifest was served from.
- **Pack origin.** A snapshot imported from a pack keeps the identity the analytical import gives
  it; the pack is recorded beside it (pack, version, build time, file SHA-256), so the same bytes
  fetched by the user and shipped in a pack are one snapshot with two recorded origins, and the
  builder's retrieval is labelled as the builder's.

## Build order

1. **First slice: Money and What residents report, City open data only.** Tables
   `budget_adopted`, `budget_actuals`, `service_reports` (closed years and one dated open
   snapshot) and `police_calls` (by year). Acceptance: question 1 without shelter beds
   (`shelter_inventory` is ASK), answered by a query that names every table and file it read.
2. **Builder (done).** In the app library behind the `pack-builder` Cargo feature, with a thin binary,
   so the app's feature set and binary are unchanged. It takes URLs, years and filters from
   `ingestion::queue::city_datasets` (one source list), writes originals with SHA-256, records the
   publisher's freshness markers (year, `Last-Modified`, `ETag`), writes Parquet copies (DuckDB's
   `parquet` feature is compiled into every build, never loaded at runtime; the app uses it only
   to read change records), and signs the manifest. Built as
   `packs::builder` and `onrecord-pack-builder`, packs `san-diego-money` and `san-diego-reports`.
3. **Conformed layer (builder side done).** One module generates the standard columns from an
   imported table, for the builder's Parquet copies and, once wired, for the app after importing
   originals. First slice:
   identity, provenance and time groups (`fiscal_year` for question 1). Place waits for the
   pre-2021 district lines. `packs::conformed` generates identity, provenance and time for the
   builder's Parquet copies; the app does not yet run it after importing originals.
4. **App import (done for the first-slice tables).** Verify the signature and every file hash before writing anything; then import
   originals through the analytical import, one table and year at a time, chosen by the user
   (`MAX_STREAMED_CELLS` is per import). Tamper tests: one byte changed in a file, and in the
   manifest, each refused. Built as `packs::import` and the data pack import panel. An import
   re-verifies the manifest and exactly the files it imports, so unchosen files may be absent.
   **Published packs (done):** `packs::published` fetches a pack's signed manifest from
   `Writerslogic/onrecord-<pack>` into `<app data>/packs/<pack>/<packVersion>/`, verifies it
   without its files, lets the user choose tables and years, downloads only those originals (each
   bounded by its listed size, kept only when size and SHA-256 match, else deleted), and imports
   them through the same path.
5. **First-run fetch (done).** On a library with no imported tables, the app offers to fetch the
   core City tables (operating budget and actuals, Get It Done open and Encampment reports, calls
   for service) straight from their publishers; it does not read a pack.
6. **Coverage (2026-10-07).** Every City open-data table above that the source list fetches is in a
   pack: `san-diego-money`, `san-diego-reports`, `san-diego-enforcement` (stops and companions,
   offences, parking citations 2012 on, collisions, hate crimes) and `san-diego-capacity`
   (development approvals). Each table has a conformed definition: the publisher's key where it
   has one, and a time column, or none for a reference list or a holdings snapshot. Publishers
   repeat their keys inside a file (2.6% of 2025 calls for service, 8 call types, a few
   approvals), so a repeated key's `record_id` is numbered `key:n` in file order and
   `publisher_record_id` keeps the key as published. Not packed: SANDAG schools and transit stops (no stated licence), the
   short-term rental registry (its original carries host names and phones, and a pack ships
   originals byte for byte), and Commission on Police Practices cases (file names differ by year).
7. **Snapshots (built 2026-10-07; first published run pending the CI secrets).** Rule 7 for the
   City files overwritten in place: `san-diego-snapshots`, extended daily by a scheduled job (see
   Snapshots below). **Later:** fetch only what was published after a pack's build date, using the
   recorded freshness markers (still later: the app fetches a published pack's files, not newer
   publisher rows); the remaining PD packs; ASK tables once permission is in writing; choosing an
   earlier snapshot of a table from a published pack (a choice downloads the latest; any snapshot
   can be imported from a local copy of the pack).

## Snapshots

**What.** The City overwrites many files in place, so their history exists nowhere else: open Get
It Done reports, City properties, parcels and leases, budget and actuals files, the budget and
calls lookups, the RIPA stop files, collisions and hate crimes. The pack `san-diego-snapshots`
follows every catalog table published as one file rather than by year
(`packs::catalog::snapshot_tables`, derived, not listed by hand: 21 tables on 2026-10-07; Get It
Done contributes only its open file, since its closed reports are by year).

**Schedule.** `.github/workflows/pack-snapshots.yml` runs daily at 13:23 UTC (and on demand):
`onrecord-pack-builder snapshot san-diego-snapshots <work-dir>` in one job, then
`scripts/publish_pack.py --incremental <work-dir>` in a second job on another runner, which
receives the work directory as an artifact. The seed reaches only the first job's snapshot step,
and no Python runs on that runner; the token reaches only the second job, whose dependencies are
installed hash-checked from `scripts/publish_pack.py.lock` (`uv run --locked --script`). Each run:

1. fetches the published manifest through `packs::published` and verifies it against the pinned
   keys. HTTP 404 on the manifest is the first run and starts an empty manifest; any other
   failure, including the 401 Hugging Face answers for a repository that does not exist, aborts.
   The owner creates the empty public repository once, by hand
   (`uv run --locked --script scripts/publish_pack.py --ensure-repo san-diego-snapshots`).
2. retrieves each file. Bytes equal (size and SHA-256) to the file's latest snapshot add a
   **check**; other bytes become a new **snapshot** (original, conformed Parquet) and, when an
   earlier snapshot exists, a **change record** against it, computed from that earlier original
   fetched back from Hugging Face and hash-checked. A file that fails before its bytes are kept
   is left out and listed in `not-snapshotted.txt`. Once its original is kept, a snapshot is
   published even if its Parquet copy or change record fails, and the table and reason go to
   `incomplete.txt`: a later snapshot without a change record means the comparison failed, not
   that nothing changed (the new snapshot itself shows the bytes differed). Either list makes the
   job fail visibly after the others are published.
3. writes the cumulative manifest (every earlier file and check plus the new ones,
   `packVersion` = the run's UTC stamp), signs it, writes `new-files.txt`, and verifies the
   manifest and every new file as the app would before exiting.
4. uploads `manifest.json`, `README.md` and the new files in one commit on top of the commit the
   manifest was read from. The script refuses a manifest that drops a published file or check,
   changes any field of a published file's entry, or lists a new file at a published path:
   published files are immutable.

**Layout.** `originals/<table>/<stamp>/<publisher file name>`,
`parquet/<table>/<stamp>/<stem>.parquet`, `changes/<table>/<stamp>.parquet`, where `<stamp>` is
the UTC retrieval time as `YYYY-MM-DDTHHMMSSZ`. Table names and conformed definitions are the
curated packs'. A change record is `PackFileFormat::Changes`; the app lists and counts it, and
shows its rows on request (`read_data_pack_change_record`, checked against the inspected manifest
and the file's SHA-256 on every read), but never imports it. Its rows: `change` (`added`, `removed`, `changed`, `column_added`, `column_removed`),
`publisher_record_id`, `key_occurrence`, `field`, `before`, `after`, `note`, all from the original
cell text, never typed values. A keyed table compares records by key and the key's repeat in file
order (so a repeated key is never multiplied); rows without a key compare by their cells in the
columns both snapshots have, with multiplicity. Past half the records changed, changed records are
listed whole as removed and added, and `note` says so. The manifest entry counts each kind
(`changes`).

**The CI key.** `pack-ci-2026-1`, public key
`3049662bc87c61409d8b8a515d60adf1420a32504bd4f6215ac0c121550a318f`, pinned in `PACK_PUBLIC_KEYS`
beside `pack-2026-1` and separate from it. Each pinned key carries the packs it signs
(`PackKeyScope`), and verification refuses any other: the CI key verifies only
`san-diego-snapshots`, the owner's Keychain key only the curated packs, so a leaked CI seed cannot
sign a curated release. A manifest's signed `keyId`, when present, must be the pinned name of the
key that verified it. The builder applies the same rule to its key before it retrieves anything,
so `build` refuses the CI seed and `snapshot` the Keychain key. The `HF_TOKEN` the job holds
should be a fine-grained token with write access to `onrecord-san-diego-snapshots` only. Its seed is the GitHub secret `ONRECORD_PACK_SEED` (and the Keychain item
`com.onrecord.pack-publisher` / `ci-seed-v1`); the builder takes it only with
`ONRECORD_PACK_KEY_ID`, and only when that exact (name, public key) pair is pinned. The signature's
`signerKeyId` is the key's SHA-256 (verification binds it to the key), so the manifest names the
key in its own signed `keyId` field.

**What a check proves, and what it does not.** That the publisher served exactly the latest
snapshot's bytes at the check time, as retrieved by the job. It does not show the file unchanged
between checks: a change made and reverted between two runs is invisible, and a snapshot's time is
when it was retrieved, not when the City changed the file (`Last-Modified` and `ETag` are recorded
as served, not trusted). A missing day means the file was not checked that day, not that it was
unchanged.

**In the app.** Choosing a table of the published snapshot pack downloads and imports its latest
snapshot only; earlier snapshots are listed and counted, never downloaded by that choice, and one
whose source the app no longer lists cannot block it. A local copy of the pack can import any
snapshot. Each imported snapshot is its own `source_id`, so `analysis_conformed_<table>` holds
every snapshot imported so far (the latest imported on two days is two sources), and a table also
imported from a curated pack (`service_reports` from `san-diego-reports`) holds those rows too: a
count over the view counts each source. The summary shows each table's unchanged checks (count and latest) and change records.

**Growth.** A check adds about 1.1 KB to `manifest.json` (pretty JSON plus its base64 canonical
copy); 21 a day is about 24 KB a day, 9 MB a year. The app's manifest download bound is 32 MiB,
about three years; before then, checks move to per-period files listed by the manifest.

## From the research blueprint (owner's file, 2026-10-06)

`San_Diego_Research_Blueprint.xlsx` (10 investigations, 13 proposed datasets, 56 sources) adds
City open-data sources that fit the Money pack under the same licence, and two dataset designs the
rules above already need:

- **Money pack additions (City open data, PDDL), added:** Capital Improvements Program budget and
  actuals by fiscal year (F03, F04) and City-owned properties, parcels and leases (R02 to R04), as
  `capital_budget`, `capital_actuals`, `city_properties`, `city_parcels` and `city_leases`
  (`docs/data-sources/city-open-data.md` section 8).
- **Not added: Goods and Services contracts (F13), awarded CIP construction contracts (F14, E10),
  cumulative amounts paid to vendors (F15).** Checked 2026-10-06: none is on data.sandiego.gov or
  carries the PDDL. The contract lists are HTML tables whose only linked files are per-contract
  PDFs; the vendor payments are one PDF per fiscal year; the relaunched catalogue (123 datasets)
  has no contracts or vendor-payment dataset.
- **Other City open data it names:** development permits (H01), completed paving and street
  segments (E07, E08), stormwater conveyance GIS (E02).
- **D13, source snapshots and change history,** is rule 7 plus the manifest's freshness markers.
- **D08, verified entities and record crosswalks,** is the `organisations` reference table: one
  evidenced identity or identifier relation over time (vendor ids, EINs, APNs, CIP numbers).
- Its investigations I06 (encampment interventions beyond report closure) and I08/I09 (contract
  lifecycle, payment timing) use the Enforcement and Money packs; its records that are documents
  (DS-450 forms, Housing Commission packets, audit reports) are collected sources, not pack tables.

## Held registry entries

Removed from `src-tauri/src/packs/sources.json` until their publisher serves them to the builder.
Every `www.huduser.gov` URL answers a non-browser client (curl, the builder) with
`HTTP/2 202`, `content-length: 0`, `x-amzn-waf-action: challenge` (probed 2026-10-07): an AWS WAF
JavaScript challenge, not the file. A browser User-Agent is served the file, but posing as a
browser to pass a bot challenge is not something the builder does. `files.hudexchange.info` is not
challenged and stays.

| Pack | Table | URL |
|---|---|---|
| ca-housing | `hud_fair_market_rents_history` | `https://www.huduser.gov/portal/datasets/FMR/FMR_All_1983_2027.csv` |
| ca-housing | `hud_small_area_fair_market_rents` | `https://www.huduser.gov/portal/datasets/fmr/fmr2027/FY27_safmrs.xlsx` (sheet `SAFMRs`) |
| ca-housing | `hud_picture_subsidized_households_pha` | `https://www.huduser.gov/portal/datasets/pictures/files/PHA_{year}_2020census.xlsx`, 2022 to 2025 (sheet `PHA_EXTRACT`) |
| ca-housing | `hud_picture_subsidized_households_project` | `https://www.huduser.gov/portal/datasets/pictures/files/PROJECT_{year}_2020census.xlsx`, 2022 to 2025 (sheet `PROJECT_EXTRACT`) |
| ca-housing | `hud_lihtc_properties` | `https://www.huduser.gov/lihtc/lihtcpub.zip` (sheet `Data`) |
| ca-homelessness-system | `hud_hic_counts_by_coc` | `https://www.huduser.gov/portal/sites/default/files/xls/2007-2025-HIC-Counts-by-CoC.xlsx` (sheet `2025`) |
| ca-homelessness-system | `hud_pit_veteran_counts_by_coc` | `https://www.huduser.gov/portal/sites/default/files/xls/2011-2025-PIT-Veteran-Counts-by-CoC.xlsx` (sheet `2025`) |
| ca-homelessness-system | `hud_hic_projects` | `https://www.huduser.gov/portal/sites/default/files/xls/2025-HIC-Counts-by-State.csv` |

The app's client refuses a 202 rather than keeping its empty body as the file.

`usaspending_hud_assistance` is held for another reason: the archive names each file by the date
it was generated, and the registry substitutes only `{year}`, so the URL keeps a literal
`{yyyymmdd}` and answers 404. It needs the current file names resolved from the archive listing.

| Pack | Table | URL |
|---|---|---|
| ca-public-money | `usaspending_hud_assistance` | `https://files.usaspending.gov/award_data_archive/FY{year}_086_Assistance_Full_{yyyymmdd}.zip`, 2007 to 2026 |

The FAC files are held because `app.fac.gov/robots.txt` is `User-agent: * / Disallow: /`, which
the app's client honours ("Portal security policy rejected the operation: 'app.fac.gov' excludes
this path through robots.txt rule '/'"):

| Pack | Table | URL |
|---|---|---|
| ca-public-money | `fac_general` | `https://app.fac.gov/dissemination/public-data/gsa/federal-fiscal-year/{year}-ffy-general.csv`, 2016 to 2027 |
| ca-public-money | `fac_federal_awards` | `https://app.fac.gov/dissemination/public-data/gsa/federal-fiscal-year/{year}-ffy-federal_awards.csv`, 2016 to 2027 |
| ca-public-money | `fac_findings` | `https://app.fac.gov/dissemination/public-data/gsa/federal-fiscal-year/{year}-ffy-findings.csv`, 2016 to 2027 |

The HCD dashboard worksheets are held because their header is not the sheet's first row: one or
two rows of explanatory text (or, in `ERF 2R `, a transposed layout) sit above it, and the
worksheet reader takes the first row of the box its values span as the header. Built as listed,
the first column of each `hcd_hhap<n>_fiscal_snapshot` was named after the banner text ("Per
Health and Safety Code 50220 ...") and `hcd_hhap4_alternative_disbursement_plans` had no key
column. They need a header-row setting in the registry.

| Pack | Table | Worksheet of `hhap-dashboard-fiscal-snapshot.xlsx` / `erf-all-rounds-encampment-projections-and-status.xlsx` |
|---|---|---|
| ca-homelessness-system | `hcd_hhap1_fiscal_snapshot` ... `hcd_hhap6_fiscal_snapshot` | `HHAP 1` ... `HHAP 6` |
| ca-homelessness-system | `hcd_hhap4_alternative_disbursement_plans` | `HHAP 4 ADPs` |
| ca-homelessness-system | `hcd_erf_active_projects_persons_served` | `All Active Projects` |
| ca-homelessness-system | `hcd_erf_status_by_round` | `ERF 2R ` |
| ca-homelessness-system | `hcd_erf_persons_served_by_county` | `Summary - by County` |

The full entries (licence evidence, headers, sizes) are in `sources.json` at commit `3a6b013`.

## Open

- Written permission, or a stated licence, from the UCSD Homelessness Hub, the SDSU sanitation
  project and the Regional Data Library.
- Pre-2021 council district boundaries.
