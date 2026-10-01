# Regional public data ingestion and analysis

The research desk includes 76 regional entry points and 42 selected housing/budget sources requested on September 13, 2026, alongside the four existing City document collections. Filter by topic or search by publisher, URL, and subject. Each entry states coverage, publisher, acquisition route, and counting considerations. A source entry is not a claim that its entire holdings are accessible or imported.

## Using the workflow

1. Choose a source in the research desk and select **Add** in the toolbar. Acquisition uses the existing managed evidence store, request limits, pause/resume controls, and host policies. Root aliases resolve to the maintained entry point; a URL for a particular dataset keeps its narrower scope.
2. Open **Analysis** and refresh acquired tables. Select a downloaded table, or browse for a local export. The acquired-file route carries its original URL, filename, and recorded SHA-256 into analysis and checks the digest before importing.
3. Use **Regional Data** for general tables. The established RIPA, calls, NIBRS, and budget families remain available. Record the observation period separately from retrieval time and leave partial coverage checked unless completeness is established.
4. Import the table. In **Data discovery**, choose that snapshot, a grouping field, an optional measure, a time field, and scope filters, then select **Discover patterns**. Use **Explore records** to inspect a group, **Open query in SQL** to modify or export a check, or **Export discovery report** to retain the source identity, scope, methods, SQL, results, and failures together. Templates and handwritten SQL remain available.

Search now defaults to all source jurisdictions. Publisher scope is resolved from the registered URL, and lexical fallback respects agency, jurisdiction, document-type, and date filters. County and Housing Commission records are no longer labeled as City records. Unknown sources remain unclassified. Collection timestamps are not presented as publication or observation dates for crawled documents. Use **Index missing** to refresh attribution on previously indexed documents; unchanged text reuses its vectors. Lexical fallback considers up to 5,000 matching requests before applying context filters.

## Guided discovery and document evidence

Data discovery works with all imported regional table formats and uses the complete filtered snapshot for its calculations. It does not require an embedding model or a remote language model. Record counts are the default; summing values and calculating an unweighted mean are explicit choices. Filters support exact values, literal case-insensitive text, and missing cells. Missing groups stay in the denominator.

| Check | Evidence returned | Interpretation |
| --- | --- | --- |
| Scope | Record count, reported/missing measurements, value range, first/last period, unusable dates | Import time never establishes data freshness. |
| Column quality | Missingness, cardinality, ranges for up to 64 columns | Suppressed or mixed values remain text rather than becoming zero. |
| Group differences | Count, observed sum or mean, missing values, share of all matching records | Shares refer to records. Sums span all selected periods and require compatible units and nonoverlapping totals. |
| Time changes | Prior/current values and record counts within each group | Differences are withheld for gaps or missing measurements; percentage change is unavailable for a zero baseline. |
| Unusual values | Source cells, peer counts, quartiles, fences, total flagged records | Review leads using 1.5 × IQR within the chosen group and period, at least eight reported values, and positive spread. These are not findings of error or wrongdoing. |
| Repeated rows | Original cells, hash, repeat count, first/last source row | Exact repeats across the whole snapshot; filters do not apply to this check. No automatic deletion or deduplication. |

The time field can be an explicit fiscal/year code or a date bucketed by year, month, or day. Two-digit year codes retain their source meaning. Gaps do not become zeros. The app does not certify comparability when methods, departmental boundaries, populations, units, or fiscal definitions change. Mean and percentage calculations are approximate derived statistics; source decimal values and sums retain the native engine's exact representation. Quartiles use DuckDB's [discrete quantile aggregate](https://duckdb.org/docs/current/sql/functions/aggregates#quantile_discx-pos), which selects observed values rather than interpolating them.

The panel shows the first 100 result rows per check; aggregation occurs before that display limit. SQL inspection can return 500 rows and CSV export can return 10,000. Empty, failed, stopped, and truncated checks are distinguished. Editing controls marks existing results as belonging to the previous scope. Exported JSON includes the immutable snapshot ID, original URL, source SHA-256, retrieval/observation metadata, chosen fields and filters, methods, SQL, returned rows, and per-check failures.

Document retrieval now searches extracted text directly through the existing persistent token index, without first installing or loading a model. Every search term must occur in a document; the excerpt centers on the tightest cluster of matching terms and may not contain every term when they occur far apart. The index covers at most four million characters per document. A missing match is not proof of absence; image-only pages require OCR first. Semantic search adds matches after explicit model initialization. Results from both methods are combined by reciprocal rank, not by mixing cosine and rank scores. Distinct documents within one request remain distinct. The evidence viewer's visibility check and context-aware masking are applied before excerpts reach the UI. **Review source record** opens the containing record for document and provenance inspection.

The native SQL boundary now traverses CTEs, nested expressions, and subqueries. It accepts the budget comparison's generated CTE names, refuses nested reads of unrelated application tables, and cites only referenced analytical snapshots, resolving an application's latest view to its actual snapshot.

Verification for this work is recorded in [data-discovery-verification-2026-09-14.json](research/data-discovery-verification-2026-09-14.json). The native discovery integration tests import real CSV fixtures and execute the exact queries generated by the UI, covering fiscal and calendar gaps, missing amounts, zero baselines, repeated rows, outliers, and denominators under a one-row output limit. Frontend tests verify the fixture stays synchronized with the builder and exercise partial failures, stopping, stale scope, and row inspection. Regenerate the shared fixtures with `node scripts/discovery-fixture.mjs --write` and `node scripts/automatic-discovery-fixture.mjs --write` after changing query construction.

## Acquisition changes

- Known regional publication sites skip unrelated NextRequest probes. The three NextRequest portals retain request API discovery, including the County’s custom hostname.
- Catalog detection is restricted to unfiltered catalog roots. Selecting a package, file, or filtered URL cannot silently enumerate the whole publisher catalog. SANDAG uses a domain-scoped Socrata catalog; California uses CKAN. California package pages expose their `package_show` companion, and selected SANDAG/County Socrata pages expose schema metadata and complete CSV export routes. [CKAN API guide](https://docs.ckan.org/en/latest/api/).
- The County budget application declares its operating and revenue dataset IDs in public configuration. Those declarations resolve to schema and CSV URLs on `data.sandiegocounty.gov`; the application hostname itself is not treated as a tabular Socrata catalog. [County budget portal](https://sandiegocounty.budget.socrata.com/).
- SANDAG’s legacy `opendata.arcgis.com` Hub is recognized. Published content group and item properties drive discovery; unrelated owner IDs and page asset hashes are not treated as datasets. Layer queries preserve explicitly selected API exports. Repeated or empty feature pages that still advertise more records report incomplete coverage. [SANDAG geographic portal](https://sdgis-sandag.opendata.arcgis.com/), [ArcGIS layer query API](https://developers.arcgis.com/rest/services-reference/enterprise/query-feature-service-layer/).
- OnBase meeting IDs and document types survive crawl deduplication. The selected meeting’s declared `Documents/ViewAgenda` endpoint is resolved without executing JavaScript. Its HTML agenda is preserved and linked attachments are discovered. PDF links commented out in the index are ignored. [Council agenda archive](https://sandiego.hylandcloud.com/211agendaonlinecouncil).
- The Granicus preset points to its public Council archive instead of the login at its root. Literal public `window.open` meeting URLs are extracted without executing scripts. Office web-viewer links resolve to their published source workbook. [Public Council video archive](https://sandiego.granicus.com/ViewPublisher.php?view_id=3).

## Table fidelity and limits

Supported analytical inputs: UTF-8 CSV, TSV, JSON object arrays, JSONL/NDJSON, GeoJSON and ArcGIS feature JSON, Socrata rows JSON with column metadata, XLSX, XLS, and ODS. JSON properties from later rows are retained. Nested values and geometry remain JSON text, including supplied coordinate-system metadata; this does not implement spatial joins. GIS shapefiles should be exported as GeoJSON or CSV for analysis.

General-table inference examines every nonempty value. Leading-zero codes, geographic identifiers, mixed types, suppression symbols, and nondetect qualifiers remain text. Whole numbers use BIGINT, supported fixed-point values use DECIMAL(38,10), and ISO dates use DATE. Values outside conservative inference limits remain text. Decimal SQL results and integers beyond JavaScript’s exact range return as strings to preserve their values. Existing specialized import families retain their own column conventions.

Each import retains a source hash, publisher URL when supplied, observation period, parser version, validation summary, original positional row ledger, and normalized rows. Separate source URLs and worksheet selections produce separate snapshots even when the underlying file bytes match. General datasets have source-specific latest views. Acquired originals remain in the managed evidence store; a standalone analytical import records the local source path and hash, so retain that original file.

Imports are bounded at 96 MiB, 750,000 rows, 512 columns, and 6,000,000 cells. Oversized files fail with instructions to split the export; they are not silently truncated. Multi-sheet workbooks require a sheet name, report available sheet names, and mark the selected-sheet snapshot partial. Formula values come from workbook caches and are not recalculated. Declared JSON continuation/transfer limits also force partial coverage. A count is a count of imported rows, not automatically a count of people, events, parcels, or votes.

The acquired-table selector displays the latest 500 completed table candidates. JSON metadata can appear among them; table parsing rejects metadata objects that do not contain rows. Profile templates inspect the first 64 columns and state that limit. Full tables remain available to explicit SQL.

SQL results display at most 500 rows by default; CSV exports use a chosen destination and include at most 10,000 rows. Partition a larger result explicitly when exporting it. These display/export caps do not reduce the rows used by aggregate queries.

FTP and SFTP are local-export routes: transfer selected files using a separate client, then import them with their original remote URL. The app does not collect file-transfer credentials. Browser challenges, account-only records, partner systems, and dashboards without public exports also require downloaded files. No video transcription, automatic login, public-records submission, or unverified private API is added.

## Verification

Deterministic tests cover every requested URL and alias, dataset scope, meeting identities, public configuration adapters, suppression and numeric precision, duplicate headers, malformed rows, geographic metadata, source hash verification, separate publishers and worksheets, acquired-file handoff, and SQL query execution. The integration tests found and fixed an existing DuckDB panic caused by reading columns before query execution.

Read-only live checks inspected SANDAG’s catalog, the County budget configuration, the SANDAG/SanGIS Hub, and the Council agenda index through a meeting and its published document. They do not start a bulk crawl. Other catalog entries describe usable public-discovery or export routes; they do not claim a passed end-to-end live connector test.

```sh
npm run test:unit
npm run build
npm run lint
cargo test --workspace --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml --test regional_live -- --ignored --nocapture
```

## Expanded regional and housing coverage

The additional routes include UCSD Homelessness Hub and its ArcGIS content group, research repositories, housing providers, education and transit agencies, federal catalogs, and news publishers. Exact aliases repair the entered UCSD and inewsource spellings. The repeated Open Data San Diego entry remains one source. Axios points to its San Diego section. Reporting has a separate category and search jurisdiction; Inside San Diego remains City communications. A catalog that republishes datasets is labeled as a multi-publisher source rather than assigning every asset to one government.

The 42 housing entries retain fiscal/edition context and evidence type. Filter proposals, adopted operating budgets, adopted capital budgets, plans, performance reports, audits, and agency statements separately. PDFs use one-page, zero-depth, one-resource acquisition recipes. The FY2026 HSSD volume III chapter covers capital projects, while the FY2025 volume II HSSD chapter covers operating activities. The SDHC FY2026 City-format file and HAR24-005 report present proposed budgets. Archive-folder dates are not assumed to be observation dates. [FY2026 HSSD CIP chapter](https://www.sandiego.gov/sites/default/files/2025-08/fy26ab_v3hssd.pdf), [SDHC proposed FY2026 budget](https://sdhc.org/wp-content/uploads/2025/01/FY26-Budget-City-Format.pdf).

Nine supplied PDF paths needed publisher-confirmed capitalization repairs. The supplied County consolidated-plan filename and Escondido blob attachment still return 404; their original identities remain available for local-file import. The County publishes a current consolidated-plan collection at its [plans and reports page](https://www.sandiegocounty.gov/content/sdc/sdhcd/nppr/plans-policies-reports.html); it is not silently substituted for the missing exact edition. Transparent Homeless Data did not respond, and the Vercel dashboard exposes a JavaScript shell. Both offer export workflows. PPIC, FOX 5, and Axios may restrict or rate-limit automated requests.

The Transportation portal uses domain-scoped Socrata discovery and selected-dataset schema/CSV companions. Data.gov's old CKAN endpoint returned 404. Its current dataset pages expose distributions and public harvest-record metadata, which the crawler recognizes as payloads for existing DCAT link discovery. The replacement GSA API requires a key and is not configured here. [Data.gov user guide](https://data.gov/user-guide/), [replacement API documentation](https://resources.data.gov/catalog-api/). The Regional Data Library publishes WordPress/Metatab resource pages; it is not treated as CKAN.

Regional presets no longer impose a host allowlist that suppresses linked file downloads. Page following still obeys the source/platform gate, crawl limits, and host policy. Explicit custom allowlists retain their existing meaning. This allows City downloads on seshat.datasd.org, ArcGIS metadata, and publisher-linked external documents.

## Operating-budget analysis

The importer now uses DuckDB appenders for table rows and the original-row ledger. It avoids duplicating common headers for every row in memory and checks flush errors before committing. These changes support the complete City operating-budget release inspected during verification: 63,825,383 bytes, 548,811 rows, and nine columns. Existing malformed-row accounting, exact decimals, source hashes, and transaction boundaries remain in place.

The operating-budget family treats budget_cycle as a category, preserves account/fund codes, and binds amounts as exact decimal strings. Its comparison template handles lowercase adopted status and separates fiscal year, fund, department, and revenue/expense account class. A per-snapshot coverage template shows missing cycles and amounts. The inspected release contains 32,596 rows with blank budget_cycle, all labeled report_fy=26. These remain unspecified; the importer never assumes they are adopted.

For the opt-in full-export check, download the publisher's [operating-budget CSV](https://seshat.datasd.org/operating_budget/budget_operating_datasd.csv), set ONRECORD_BUDGET_EXPORT to the local path, then run:

```sh
ONRECORD_BUDGET_EXPORT=/path/to/budget_operating_datasd.csv cargo test --manifest-path src-tauri/Cargo.toml --test regional_analysis full_city_budget_export_reaches_sql_without_truncation -- --ignored --nocapture
```

This test imports into a temporary database and checks that SQL and the original-row ledger contain every source row. It does not modify the production evidence library.

## City OpenGov and budget-versus-actual comparisons

The budget.sandiego.gov preset resolves its public OpenGov bootstrap, six publicly listed report metadata endpoints, and supporting document links without evaluating scripts. Tenant identity and public visibility are checked; private or undeclared report IDs are not enumerated. Signed supporting-document URLs are used as published and may expire. A selected unfiltered report hash resolves to that report's metadata; filtered visual states require an exported spreadsheet so selection is not discarded.

This preset also seeds the City's [Finance collection](https://data.sandiego.gov/departments/department-of-finance/), which publishes ten budget, actuals, capital, and reference datasets. The raw releases remain distinct from OpenGov's rendered views. Its published report metadata is not itself a table of expenditures. [City budget portal](https://budget.sandiego.gov/transparency).

In **Analysis → Budget versus actuals**, explicitly select two imported snapshots, a fiscal-year code, adopted/proposed cycle, expense/revenue account class, and optional fund. The builder aggregates each file to fiscal year + fund + funds center + account before a full outer join, avoiding multiplication when either file has several rows per key. It retains row counts, unmatched keys, and incomplete amounts. Variance is actual minus budget; percentage variance divides by the absolute budget and remains empty for a zero denominator or incomplete pair. Budgets cannot be selected as actuals merely because they contain amount columns. SQL stays visible and exportable. Coverage templates expose the years and cycles actually present.

The complete operating-actuals release also passed temporary-database ingestion: 69,104,357 bytes, 663,858 rows, eight columns, zero rejected rows. Despite its recent file publication date, this snapshot contains fiscal-year codes 11 through 23 only. The build's comparison query was executed against both complete releases for FY23 expense accounts: 23,968 matched account groups, 6,307 budget groups without actuals, and 28,382 actuals groups without the selected budget baseline. These are key-matching results, not findings of missing money or program effectiveness. No unmatched amount is silently replaced with zero.

The full budget import took about 54 seconds in this environment and reached approximately 2 GiB maximum resident memory. Large imports need working memory beyond the source file size; source bytes, rows, columns, and cells remain bounded. Parser version is included in snapshot identity so revised normalization does not overwrite earlier parser snapshots.

Additional opt-in checks:

```sh
# Requires a DuckDB CLI executable; set DUCKDB_CLI if it is not on PATH.
npm run test:budget-comparison
ONRECORD_ACTUALS_EXPORT=/path/to/actuals_operating_datasd.csv cargo test --manifest-path src-tauri/Cargo.toml --test regional_analysis full_city_actuals_export_preserves_observed_fiscal_years -- --ignored --nocapture
```

The comparison regression executes the actual UI query builder with duplicate keys, missing counterparts, null amounts, zero budgets, separate account classes, and unsafe input strings. Verification observations are recorded in [the expansion audit](research/regional-source-expansion-2026-09-13.json).

## Source routes

118 maintained routes below supplement the four existing City collections, for 122 research-desk entries. Public discovery is bounded acquisition, not a claim of complete holdings or successful live ingestion.

| Source | Publisher | Topic | Evidence / period | Route |
| --- | --- | --- | --- | --- |
| [Air Pollution Control District data](https://www.sdapcd.org/content/sdapcd/apcd-air-data.html) | San Diego County Air Pollution Control District | Environment and water | Source collection | Public discovery |
| [Regional WaterGIS](https://sdirwmp.org/watergis) | San Diego Integrated Regional Water Management | Environment and water | Source collection | Public discovery |
| [Project Clean Water](https://projectcleanwater.org) | Project Clean Water | Environment and water | Source collection | Public discovery |
| [Think Blue by the numbers](https://thinkblue.org/by-the-numbers) | City of San Diego Think Blue | Environment and water | Source collection | Public discovery |
| [San Diego Regional Water Board](https://www.waterboards.ca.gov/sandiego) | California Regional Water Quality Control Board, San Diego Region | Environment and water | Source collection | Public discovery |
| [City campaign disclosure filings](https://ssl.netfile.com/static/agency/csd/) | City of San Diego / NetFile | Elections and finance | Source collection | Public discovery |
| [County Registrar of Voters](https://www.sdvote.com) | County Registrar of Voters | Elections and finance | Source collection | Public discovery |
| [Independent Budget Analyst reports](https://www.sandiego.gov/iba/reports) | City Independent Budget Analyst | Elections and finance | Source collection | Public discovery |
| [City retirement system reports](https://www.sdcers.gov) | San Diego City Employees’ Retirement System | Elections and finance | Source collection | Public discovery |
| [County retirement association reports](https://www.sdcera.org) | San Diego County Employees Retirement Association | Elections and finance | Source collection | Public discovery |
| [Assessor / Recorder / County Clerk](https://www.sdarcc.gov) | County Assessor / Recorder / County Clerk | Government | Source collection | Public discovery |
| [San Diego Unified budget](https://www.sandiegounified.org/about/budget) | San Diego Unified School District | Education and economy | Source collection | Public discovery |
| [City electronic disclosure portal](https://efile.sandiego.gov) | City of San Diego | Elections and finance | Source collection | Public discovery |
| [UCSD MassIVE file exports](ftp://massive-ftp.ucsd.edu) | UC San Diego MassIVE | Research | Source collection | Local export |
| [SanGIS FTP exports](ftp://ftp.sangis.org) | SanGIS | Geography | Source collection | Local export |
| [City FTP exports](ftp://ftp.sandiego.gov) | City of San Diego | Government | Source collection | Local export |
| [City SFTP exports](sftp://sftp.sandiego.gov) | City of San Diego | Government | Source collection | Local export |
| [City open data](https://data.sandiego.gov) | City of San Diego | Open data | Source collection | Public discovery |
| [SANDAG open data](https://opendata.sandag.org) | SANDAG | Open data | Source collection | Public discovery |
| [City reports and records](https://www.sandiego.gov) | City of San Diego | Government | Source collection | Public discovery |
| [County budget portal](https://sandiegocounty.budget.socrata.com) | County of San Diego | Open data | Source collection | Public discovery |
| [County reports and records](https://www.sandiegocounty.gov) | County of San Diego | Government | Source collection | Public discovery |
| [SANDAG reports](https://www.sandag.org) | SANDAG | Government | Source collection | Public discovery |
| [City public-records requests](https://sandiego.nextrequest.com) | City of San Diego | Public records | Source collection | Public discovery |
| [Housing Commission requests](https://sandiegohousingcommission.nextrequest.com) | San Diego Housing Commission | Public records | Source collection | Public discovery |
| [County public-records requests](https://pra.sandiegocounty.gov) | County of San Diego | Public records | Source collection | Public discovery |
| [SANDAG / SanGIS geography](https://sdgis-sandag.opendata.arcgis.com) | SANDAG / SanGIS | Geography | Source collection | Public discovery |
| [UC San Diego digital collection](https://library.ucsd.edu/dc/collection/bb7998853b) | UC San Diego Library | Research | Source collection | Local export |
| [Regional Freight Viewer](https://gis.sandiego.org/FreightViewer) | Regional Freight Viewer | Geography | Source collection | Local export |
| [County community health statistics](https://www.sandiegocounty.gov/content/sdc/hhsa/programs/phs/community_health_statistics.html) | County HHSA | Health and housing | Source collection | Public discovery |
| [ARJIS public information](https://arjis.org) | ARJIS | Government | Source collection | Public discovery |
| [County education dashboards](https://data.sdcoe.net) | San Diego County Office of Education | Education and economy | Source collection | Public discovery |
| [California open data](https://data.ca.gov) | State of California | Open data | Source collection | Public discovery |
| [Regional Task Force on Homelessness](https://rtfhsd.org) | Regional Task Force on Homelessness | Health and housing | Source collection | Public discovery |
| [211 San Diego](https://211sandiego.org) | 211 San Diego | Health and housing | Source collection | Public discovery |
| [Live Well regional data hub](https://livewellsd.org/regional-data-hub) | Live Well San Diego | Health and housing | Source collection | Public discovery |
| [Innovate78](https://innovate78.com) | Innovate78 | Education and economy | Source collection | Public discovery |
| [Open Data San Diego explorer](https://opendatasandiego.com) | Open Data San Diego | Open data | Source collection | Public discovery |
| [Escondido public records](https://www.escondido.gov) | City of Escondido | Government | Source collection | Public discovery |
| [SDSU Human Dynamics](https://humandynamics.sdsu.edu) | San Diego State University | Research | Source collection | Public discovery |
| [Sheriff open data](https://www.sdsheriff.gov/resources/open-data) | San Diego County Sheriff | Government | Source collection | Public discovery |
| [Downtown San Diego Partnership](https://downtownsandiego.org) | Downtown San Diego Partnership | Health and housing | Source collection | Public discovery |
| [HomelessData exports](https://homelessdata.com) | HomelessData | Health and housing | Source collection | Local export |
| [Council agenda archive](https://sandiego.hylandcloud.com/211agendaonlinecouncil) | City Clerk | Public records | Source collection | Public discovery |
| [Council video archive and agendas](https://sandiego.granicus.com/ViewPublisher.php?view_id=3) | CityTV / City Clerk | Public records | Source collection | Public discovery |
| [UCSD Homelessness Hub](https://homelessnesshub.ucsd.edu) | UC San Diego Homelessness Hub | Research | Source collection | Public discovery |
| [San Diego Regional Data Library](https://data.sandiegodata.org) | San Diego Regional Data Library | Open data | Source collection | Public discovery |
| [Voice of San Diego](https://voiceofsandiego.org) | Voice of San Diego | News and reporting | Source collection | Public discovery |
| [Inside San Diego](https://www.insidesandiego.org) | City of San Diego | Government | Source collection | Public discovery |
| [El Cajon Collaborative](https://www.elcajoncollaborative.org) | El Cajon Collaborative | Health and housing | Source collection | Public discovery |
| [Transparent Homeless Data](https://transparenthomelessdata.com) | Transparent Homeless Data | Health and housing | Source collection | Local export |
| [KPBS](https://www.kpbs.org) | KPBS Public Media | News and reporting | Source collection | Public discovery |
| [San Diego Homelessness Analytics dashboard](https://san-diego-homelessness-analytics.vercel.app) | San Diego Homelessness Analytics | Health and housing | Source collection | Local export |
| [Public Policy Institute of California](https://www.ppic.org) | Public Policy Institute of California | Research | Source collection | Public discovery |
| [FOX 5 San Diego](https://fox5sandiego.com) | FOX 5 San Diego | News and reporting | Source collection | Public discovery |
| [CalMatters](https://calmatters.org) | CalMatters | News and reporting | Source collection | Public discovery |
| [County District Attorney](https://www.sdcda.org) | San Diego County District Attorney | Government | Source collection | Public discovery |
| [UCSD Homelessness Hub GIS](https://hhubsandiego-ucsdonline.hub.arcgis.com) | UC San Diego Homelessness Hub | Geography | Source collection | Public discovery |
| [Times of San Diego](https://timesofsandiego.com) | Times of San Diego | News and reporting | Source collection | Public discovery |
| [CBS 8](https://www.cbs8.com) | CBS 8 | News and reporting | Source collection | Public discovery |
| [California Housing and Community Development](https://www.hcd.ca.gov) | California Department of Housing and Community Development | Health and housing | Source collection | Public discovery |
| [Lucky Duck Foundation](https://www.luckyduckfoundation.org) | Lucky Duck Foundation | Health and housing | Source collection | Public discovery |
| [inewsource](https://inewsource.org) | inewsource | News and reporting | Source collection | Public discovery |
| [San Diego Rescue Mission](https://www.sdrescue.org) | San Diego Rescue Mission | Health and housing | Source collection | Public discovery |
| [The Coast News](https://thecoastnews.com) | The Coast News Group | News and reporting | Source collection | Public discovery |
| [San Diego Voice & Viewpoint](https://sdvoice.info) | San Diego Voice & Viewpoint | News and reporting | Source collection | Public discovery |
| [Axios San Diego](https://www.axios.com/local/san-diego) | Axios | News and reporting | Source collection | Public discovery |
| [Poway Unified](https://www.powayusd.com) | Poway Unified School District | Education and economy | Source collection | Public discovery |
| [HUD housing and homelessness data](https://www.hud.gov) | U.S. Department of Housing and Urban Development | Health and housing | Source collection | Public discovery |
| [San Diego Housing Commission](https://sdhc.org) | San Diego Housing Commission | Health and housing | Source collection | Public discovery |
| [Metropolitan Transit System](https://www.sdmts.com) | San Diego Metropolitan Transit System | Transportation | Source collection | Public discovery |
| [UCSD Data Nexus](https://datanexus.ucsd.edu) | UC San Diego | Research | Source collection | Public discovery |
| [Data.gov catalog](https://catalog.data.gov) | Data.gov / contributing publishers | Open data | Source collection | Public discovery |
| [U.S. Transportation open data](https://data.transportation.gov) | U.S. Department of Transportation | Transportation | Source collection | Public discovery |
| [SDSU institutional research](https://asir.sdsu.edu) | San Diego State University | Education and economy | Source collection | Public discovery |
| [City budget transparency and Finance datasets](https://budget.sandiego.gov/transparency) | City of San Diego | Housing budgets and outcomes | Collection | Public discovery |
| [FY2025 City agencies budget](https://www.sandiego.gov/sites/default/files/2024-08/fy25ab_v1cityagencies.pdf) | City of San Diego | Housing budgets and outcomes | Adopted budget · FY2025 | Selected PDF |
| [Annual action plan — August 2024 release](https://www.sandiego.gov/sites/default/files/2024-08/annual-action-plan.pdf) | City of San Diego | Housing budgets and outcomes | Plan · August 2024 release | Selected PDF |
| [FY2026 guide to the budget](https://www.sandiego.gov/sites/default/files/2025-08/fy26ab_v2guidetothebudget.pdf) | City of San Diego | Housing budgets and outcomes | Budget explanation · FY2026 | Selected PDF |
| [IBA legacy budget guide](https://www.sandiego.gov/sites/default/files/legacy/iba/pdf/budgetguide.pdf) | City Independent Budget Analyst | Housing budgets and outcomes | Budget explanation · Legacy edition | Selected PDF |
| [City operating budget dataset](https://data.sandiego.gov/datasets/operating-budget) | City Department of Finance | Housing budgets and outcomes | Dataset · Multiple fiscal years | Public discovery |
| [FY2026 City budget overview](https://www.sandiego.gov/sites/default/files/2025-08/fy26ab_v1cbo.pdf) | City of San Diego | Housing budgets and outcomes | Adopted budget · FY2026 | Selected PDF |
| [IBA budget guide — October 2024](https://www.sandiego.gov/sites/default/files/2024-10/iba-budget-guide-eng.pdf) | City Independent Budget Analyst | Housing budgets and outcomes | Budget explanation · October 2024 edition | Selected PDF |
| [FY2026 City agencies budget](https://www.sandiego.gov/sites/default/files/2025-08/fy26ab_v1cityagencies.pdf) | City of San Diego | Housing budgets and outcomes | Adopted budget · FY2026 | Selected PDF |
| [RTFH 2024 data and performance report](https://www.rtfhsd.org/wp-content/uploads/2025/07/2024-Data-Performance-Report_v3.pdf) | Regional Task Force on Homelessness | Housing budgets and outcomes | Performance report · 2024 · version 3 | Selected PDF |
| [County consolidated and annual plans](https://www.sandiegocounty.gov/content/dam/sdc/sdhcd/new-docs/cd/fy25-29_conplan_fy25-26_annualplan_final.pdf) | County of San Diego | Housing budgets and outcomes | Plan · FY2025–2029 / FY2025–2026 | Local export |
| [Public guide to the FY2026 adopted budget](https://www.sandiego.gov/sites/default/files/2025-12/publics-guide-to-the-budget-process-and-the-fy-2026-adopted-budget.pdf) | City Independent Budget Analyst | Housing budgets and outcomes | Budget explanation · FY2026 · December 2025 edition | Selected PDF |
| [FY2025 consolidated-plan annual action plan](https://www.sandiego.gov/sites/default/files/2024-07/fy25cpaap_1.pdf) | City of San Diego | Housing budgets and outcomes | Plan · FY2025 | Selected PDF |
| [FY2021 CAPER attachment 3](https://www.sandiego.gov/sites/default/files/cosdfy21caper_att3.pdf) | City of San Diego | Housing budgets and outcomes | Performance report · FY2021 · attachment 3 | Selected PDF |
| [County health: persons experiencing homelessness](https://www.sandiegocounty.gov/content/dam/sdc/hhsa/programs/phs/CHS/Persons%20Experiencing%20Homelessness%20in%20SDC%202024-2026.pdf) | County HHSA | Housing budgets and outcomes | Health report · 2024–2026 edition | Selected PDF |
| [FY2027 sheltering options announcement](https://www.sandiego.gov/sites/default/files/2026-06/2026-6-16-city-of-san-diego-funds-2-700-sheltering-options-for-individuals-experiencing-homelessness-in-fy-2027-budget.pdf) | City of San Diego | Housing budgets and outcomes | Agency statement · FY2027 · June 16, 2026 | Selected PDF |
| [FY2026 HSSD capital improvements](https://www.sandiego.gov/sites/default/files/2025-08/fy26ab_v3hssd.pdf) | City of San Diego | Housing budgets and outcomes | Adopted capital budget · FY2026 | Selected PDF |
| [FY2026 CAPER](https://www.sandiego.gov/sites/default/files/2026-08/fy2026-caper.pdf) | City of San Diego | Housing budgets and outcomes | Performance report · FY2026 | Selected PDF |
| [City HOME-ARP allocation plan](https://www.sandiego.gov/sites/default/files/cosd_home_arp.pdf) | City of San Diego | Housing budgets and outcomes | Plan · HOME-ARP | Selected PDF |
| [IBA FY2024 homelessness programs and funding](https://www.sandiego.gov/sites/default/files/23-21_fy_2024_homelessness_programs_and_funding.pdf) | City Independent Budget Analyst | Housing budgets and outcomes | Funding analysis · FY2024 · report 23-21 | Selected PDF |
| [FY2026 capital improvements budget volume](https://www.sandiego.gov/sites/default/files/2025-08/fy26ab_v3.pdf) | City of San Diego | Housing budgets and outcomes | Adopted capital budget · FY2026 | Selected PDF |
| [SDHC FY2026 proposed budget — City format](https://sdhc.org/wp-content/uploads/2025/01/FY26-Budget-City-Format.pdf) | San Diego Housing Commission | Housing budgets and outcomes | Budget proposal · FY2026 | Selected PDF |
| [SDHC FY2024 budget report](https://sdhc.org/wp-content/uploads/2023/05/SDHC-FY2024-Budget-Report.pdf) | San Diego Housing Commission | Housing budgets and outcomes | Budget document · FY2024 | Selected PDF |
| [SDHC FY2025 proposed budget report](https://sdhc.org/wp-content/uploads/2024/06/HAR24-005-SDHC-FY25-Budget-Report.pdf) | San Diego Housing Commission | Housing budgets and outcomes | Budget proposal · FY2025 · HAR24-005 | Selected PDF |
| [IBA FY2025 homelessness funding — revised](https://www.sandiego.gov/sites/default/files/2024-09/24-24-rev-fy-2025-homelessness-programs-and-funding_rev.pdf) | City Independent Budget Analyst | Housing budgets and outcomes | Funding analysis · FY2025 · report 24-24 revised | Selected PDF |
| [City FY2025–2029 consolidated-plan materials](https://www.sandiego.gov/cdbg/general/consolidatedplanmaterials/fy25-29) | City of San Diego | Housing budgets and outcomes | Collection · FY2025–2029 | Public discovery |
| [Corporation for Supportive Housing](https://www.csh.org) | Corporation for Supportive Housing | Health and housing | Research | Public discovery |
| [RTFH reports and data collection](https://rtfhsd.org/reports-data) | Regional Task Force on Homelessness | Housing budgets and outcomes | Collection | Public discovery |
| [SDHC homelessness solutions at a glance](https://sdhc.org/wp-content/uploads/2022/12/SDHC-at-a-glance-homelessness-solutions.pdf) | San Diego Housing Commission | Housing budgets and outcomes | Program summary · December 2022 path; verify document edition | Selected PDF |
| [State audit 2023-102.2: homelessness](https://information.auditor.ca.gov/reports/2023-102.2/index.html) | California State Auditor | Housing budgets and outcomes | Audit · Report 2023-102.2 | Public discovery |
| [FY2027 City financial summary](https://www.sandiego.gov/sites/default/files/2026-08/fy27ab_v1financialsummary.pdf) | City of San Diego | Housing budgets and outcomes | Adopted budget · FY2027 | Selected PDF |
| [City homelessness and sheltering trend announcement](https://www.insidesandiego.org/san-diego-homelessness-down-135-city-sheltering-options-and-supportive-housing-grow-91-2019) | City of San Diego | Housing budgets and outcomes | Agency statement · Verify comparison periods in article | Public discovery |
| [FY2025 Citywide budget](https://www.sandiego.gov/sites/default/files/2024-08/fy25ab_v2citywide.pdf) | City of San Diego | Housing budgets and outcomes | Adopted budget · FY2025 | Selected PDF |
| [RTFH HMIS newsletter — December 2024](https://www.rtfhsd.org/wp-content/uploads/2025/01/RTFH-HMIS-Data-Newsletter-Dec-2024.pdf) | Regional Task Force on Homelessness | Housing budgets and outcomes | Performance report · December 2024 | Selected PDF |
| [County draft housing element — strikeout edition](https://www.sandiegocounty.gov/content/dam/sdc/pds/GPUpdate2021/DraftHousingElement-StrikeoutVersion.pdf) | County of San Diego | Housing budgets and outcomes | Draft plan · 2021 update path; verify planning cycle | Selected PDF |
| [City consolidated plan — 2005 archive](https://www.sandiego.gov/sites/default/files/legacy/planning/community/pdf/cpc/agendas/cplan2005.pdf) | City of San Diego | Housing budgets and outcomes | Historical plan · 2005 archive | Selected PDF |
| [FY2025 Homelessness Strategies and Solutions](https://www.sandiego.gov/sites/default/files/2024-08/fy25ab_v2hssd.pdf) | City of San Diego | Housing budgets and outcomes | Adopted budget · FY2025 | Selected PDF |
| [County fifth-cycle adopted housing element](https://www.hcd.ca.gov/housing-elements/docs/san-diego-cou-5th-adopted050713.pdf) | California HCD / County of San Diego | Housing budgets and outcomes | Historical plan · Fifth cycle · May 7, 2013 edition | Selected PDF |
| [FY2027 City Auditor draft budget](https://www.sandiego.gov/sites/default/files/2026-06/fy27-draft-budget.pdf) | City Auditor | Housing budgets and outcomes | Budget proposal · FY2027 | Selected PDF |
| [FY2026 Convention Center budget](https://www.sandiego.gov/sites/default/files/2025-08/fy26ab_v2conventioncenter.pdf) | City of San Diego | Housing budgets and outcomes | Adopted budget · FY2026 | Selected PDF |
| [County Housing Blueprint](https://engage.sandiegocounty.gov/housing-blueprint) | County of San Diego | Housing budgets and outcomes | Collection | Public discovery |
| [RTFH 2024 PIT count — revised city breakdown](https://www.rtfhsd.org/wp-content/uploads/2024/09/2024-PITC-Regional-Cities-Breakdown-revised-Final-June-17-1.pdf) | Regional Task Force on Homelessness | Housing budgets and outcomes | Point-in-Time count · 2024 · revised June 17 edition | Selected PDF |
| [Escondido meeting attachment 002](https://mccmeetingspublic.blob.core.usgovcloudapi.net/escondidca-meet-4c27edfbde4e4fbe81564d7925ed98fb/item-attachment-002-2aa4e015c04f476fadae609b490f75e4.pdf) | Escondido meeting archive | Housing budgets and outcomes | Meeting attachment · Verify meeting and item in document | Local export |
