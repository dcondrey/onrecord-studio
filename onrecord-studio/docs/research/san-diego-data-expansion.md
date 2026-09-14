# San Diego data expansion: evidence, analytical data, and retrieval research

The strongest next investment is a connected collection of **San Diego surveillance records, procurement documents, audit findings, legislative decisions, and police oversight records**. These can answer questions the current source presets cannot reliably answer together: what was authorized, what was purchased, what reviewers found, and what happened afterward. Add police and budget tables alongside those documents so numerical questions have reproducible answers.

The online search covered Zenodo, arXiv, GitHub, Wolfram, official local and state publishers, university repositories, and civil-society datasets. It produced a [46-entry source catalog](san-diego-source-catalog.csv), including useful additions, existing sources to extend, evaluation resources, and candidates to defer. The ranking is an engineering judgment based on local relevance, authority, usable access, maintenance burden, and the questions each source could support. It is not a measured claim about improved application accuracy.

Research and endpoint checks were conducted on September 13, 2026. The scope is City of San Diego and SDPD first. County, statewide, and national sources have explicit jurisdiction labels. This investigation did not load records into the application, train a model, or implement semantic search.

## What the application can currently use

In this checkout, the implemented corpus is DuckDB-backed `deterministic-corpus-v3`, schema version 7. It stores document segments, term postings, similarity fingerprints, entity mentions, and relationships. Segments target 2,048 characters with 192 characters of overlap. I did not find an implemented embedding model or approximate-nearest-neighbor vector store. See [the corpus implementation](../../src-tauri/src/storage/corpus.rs) and [the existing architecture proposal](../investigative-workbench-architecture.md).

That distinction changes the data plan. Documents can improve the existing text index after successful acquisition and extraction. CSV statistics need typed tables and query support. A future vector index should be a rebuildable projection of selected document text, preserving the original source and citation coordinates. Simply converting every spreadsheet row into prose would make counts harder to audit and create many nearly identical search results.

The previous San Diego work already introduced CPP meetings, CPP resources, SDPD policies, and current Council agendas. Their appearance in this report means extending historical coverage and linking related records, rather than counting the same presets as new capabilities. The current limitations are documented in [the San Diego focus notes](../san-diego-focus.md).

## Recommended order

“P1” means the next focused implementation round; “P2” means a useful extension after that foundation; “P3” means optional, deferred, or unsuitable as local evidence. Catalog IDs identify the exact source and access caveats.

| Order | Collection | Why it earns priority | Catalog |
| --- | --- | --- | --- |
| 1 | Surveillance reports, proposed policies, and board recommendations | Connect technology names to reporting, review, and policy language | SD01–02 |
| 2 | Contracts and award records | Connect the same technology or vendor to a contract, amount, and justification | SD05–07 |
| 3 | Audits, management responses, and follow-up | Establish what was recommended and whether a later source records implementation | SD03–04 |
| 4 | Council archives, enacted instruments, and staff reports | Distinguish a proposal from an actual decision | SD08–11 |
| 5 | SDPD disclosures and relevant DA review letters | Connect released incident records to findings and subsequent prosecutorial review | SD12, SD45 |
| 6 | RIPA, calls, offenses, budgets, and dictionaries | Support exact statistical questions using typed data | SD15–18 |
| 7 | Local geography and Census context | Prevent jurisdiction errors and make geographic comparisons interpretable | SD19–20 |
| 8 | Local evaluation set, supported by research benchmarks | Measure whether new data and retrieval changes actually help | SD36–39 |

Start with a surveillance investigation that crosses the first four collections. This is a bounded demonstration of the app's intended value and exercises document linking, dates, versions, amounts, and citations at once.

## Local records with the greatest expected value

### Surveillance, spending, and decisions

The Privacy Advisory Board publishes annual surveillance material and recommendations, including amended ALPR records. Its separate policies page exposes documents under review. These are unusually relevant additions because they supply several versions of the same subject and explicit review documents. Retain the publisher's document title and date, and distinguish the board's recommendation from an adopted policy. [PAB reports](https://www.sandiego.gov/pab/reports), [policies under review](https://www.sandiego.gov/pab/policies).

A concrete starter set is the [2025 SDPD annual surveillance report](https://www.sandiego.gov/sites/default/files/2025-04/sdpd-annual-surveillance-report-2025.pdf), [amended ALPR annual report](https://www.sandiego.gov/sites/default/files/2025-08/amended-alpr-annual-report.pdf), [amended use policy](https://www.sandiego.gov/sites/default/files/2025-08/amended-alpr-sup.pdf), and [2026 PAB recommendation concerning the 2025 ALPR report](https://www.sandiego.gov/sites/default/files/2026-05/pab-final-recommendation-alpr-2025-asr-00235352xbde34.pdf). These are discovered acquisition seeds; this research has not validated every PDF's extracted text.

The City's fiscal-year contract pages expose award rows and linked documents. The goods-and-services pages describe a threshold of more than $25,000; construction has a separate collection. Store contract number, vendor, award date, fiscal year, amount, and document relationship. An award amount should never silently become a claim about payments or total program cost. [FY2026 goods and services](https://www.sandiego.gov/purchasing/bids-contracts/contracts2026), [FY2027 goods and services](https://www.sandiego.gov/purchasing/bids-contracts/contracts2027), [FY2026 construction](https://www.sandiego.gov/cip/reports/constructioncontractsfy26).

Pair these with the Auditor's report archive and recommendation follow-up material. The [SDPD body-camera audit](https://www.sandiego.gov/sites/default/files/23-001_sdpd_bodycams.pdf) is a concrete starting document. A useful application answer would show an original finding, the associated management response, and the most recent relevant follow-up, with separate citations. An old finding alone cannot establish that a recommendation remains unresolved. [Audit reports](https://www.sandiego.gov/auditor/reports), [recommendation follow-up](https://www.sandiego.gov/auditor/reports/recommendation-follow-dashboard).

### Legislative history and oversight

The City Clerk points to a Council archive covering April 20, 2009 onward, with an older archive available separately. The archive is a stronger historical foundation than the current-agenda page alone. Its portal still needs a dedicated acquisition check for pagination, attachments, and completeness. [Official documents and archive links](https://www.sandiego.gov/city-clerk/officialdocs), [Hyland Council archive](https://sandiego.hylandcloud.com/211agendaonlinecouncil).

Add resolutions and ordinances, Council reports, and local governing documents. Each serves a different evidentiary purpose: staff reports explain proposals; minutes and enacted instruments establish actions; code compilations provide reference text. Preserve those roles in retrieval results. [Resolutions and ordinances](https://www.sandiego.gov/city-clerk/officialdocs/council-resolutions-ordinances), [reports to Council](https://www.sandiego.gov/city-clerk/officialdocs/council-reports), [Municipal Code](https://www.sandiego.gov/city-clerk/officialdocs/municipal-code).

CPP is already a baseline source, but historical reports and response letters deserve explicit completeness checks. For example, the [July–December 2025 semiannual report](https://www.sandiego.gov/sites/default/files/2026-04/cpp-july-december-2025-semi-annual-report.pdf) has a reporting period different from its 2026 publication path. Store both dates. [CPP reports and resources](https://www.sandiego.gov/cpp/reports).

### Incident disclosures and later review

The SDPD mandatory-disclosure page contains a paginated case list. A live case-page check exposed links to redacted documents, photographs, audio, and video. This is an actionable source, although attachment extraction and complete crawling remain untested here. Start with document PDFs; introduce media transcription after reliable citations and timestamps exist. Some links contain embedded whitespace that an adapter must handle carefully. [SDPD SB16/SB1421/AB748 disclosures](https://www.sandiego.gov/police/data-transparency/mandated-disclosures/sb16-sb1421-ab748).

The District Attorney's OIS collection adds review letters and incident/agency metadata. Its review concerns criminal liability, and expressly does not determine agency-policy compliance or civil liability. Join an SDPD incident to a DA letter only when the identifiers and incident details support the match. Preserve the DA as a separate issuer and decision-maker. [DA officer-involved shooting reviews](https://www.sdcda.org/office/officer-involved-shootings).

The Sheriff's transparency collection can support a later County workspace, but detention statistics and Sheriff's incidents should not be pooled into SDPD totals. Geography alone does not identify the responsible agency. [Sheriff transparency reports](https://www.sdsheriff.gov/resources/transparency-reports).

## Statistical data: useful when the app can calculate

### RIPA and police activity

The City publishes RIPA base data and related tables for stop reasons, searches, actions, outcomes, and perceived characteristics. Its documentation warns that stop identifiers can repeat between historical and current files. A person ID identifies someone within a stop, rather than a persistent identity across encounters. Use collection/schema family, agency, stop identifier, and within-stop person identifier together; validate uniqueness before relying on a key. [RIPA base data](https://data.sandiego.gov/datasets/police-ripa-stops/).

Related tables can have multiple rows per person. A direct many-to-many join can inflate search or outcome counts. Aggregate each relationship at its intended unit before combining results, and keep an explicit denominator in every analytical answer. [Search-basis table and dictionary](https://data.sandiego.gov/datasets/police-ripa-search-basis/).

A bounded download confirmed that the current base CSV uses uppercase fields including `UID`, `ID`, `ORI`, and `PID`. Do not assume the portal's displayed dictionary exactly matches every current or historical file. The saved [verification record](san-diego-source-verification.json) contains actual headers and response metadata, without copying incident rows.

Calls for service provide annual CSVs and supporting call-type, disposition, and priority definitions. The public dataset excludes sensitive categories described by the publisher, so it cannot support a claim about all police calls. Calls are also a different counting unit from crimes. [Calls for service](https://data.sandiego.gov/datasets/police-calls-for-service/).

NIBRS provides offense data with its own dictionary. Preserve case and offense identifiers and the distinction between occurrence and approval dates. Count incidents only after a documented grouping rule; do not label every offense row a separate incident. [NIBRS data](https://data.sandiego.gov/datasets/police-nibrs/).

### Budgets, geography, and population

Operating-budget CSVs contain fiscal-year and budget-cycle fields, plus department, fund, account, and amount information. Load the related dimension tables so codes resolve consistently. A proposed budget, adopted budget, and another published cycle should remain distinguishable. The data are useful for allocation questions; they do not establish actual spending by themselves. [Operating budgets](https://data.sandiego.gov/datasets/operating-budget/), [department definitions](https://data.sandiego.gov/datasets/departments-city-budget/), [fund definitions](https://data.sandiego.gov/datasets/funds-city-budget/).

The City's police-beat collection includes a neighborhood crosswalk and geographic formats. Add boundary vintages to the data model before mapping historical records. SanGIS also exposes regional layers, but its own distribution terms may differ from the City portal's license. [Police beats](https://data.sandiego.gov/datasets/gis-police-beats/), [City SanGIS catalog](https://data.sandiego.gov/departments/sangis/), [SanGIS REST services](https://gis.sangis.org/maps/rest/services).

Census ACS five-year estimates can supply population context, with margins of error and a declared vintage. The current Census example documentation says data queries require an API key; an authenticated query was not run. Resident population is a useful contextual denominator, but it is not a direct measure of who drives through an area or is exposed to police stops. [ACS five-year releases](https://www.census.gov/data/developers/data-sets/acs-5year.html), [current API examples](https://api.census.gov/data/2024/acs/acs5/examples.html).

Get It Done is a useful P2 expansion for municipal services. Its dictionary supplies parent-request relationships and geographic fields. The City explicitly says these are submitted reports, not a maintenance-work record; duplicates and unverified conditions exist. A closed or referred request must not become an unsupported claim that physical work was completed. [Get It Done data and limitations](https://data.sandiego.gov/datasets/get-it-done-reports/).

### What the endpoint checks establish

Four CSV endpoints returned HTTP 206 for bounded 4,096-byte requests. The response headers reported the following full-object sizes at the time of checking:

| File | Bytes reported by server |
| --- | ---: |
| Current RIPA base | 75,166,145 |
| 2026 calls for service | 31,530,569 |
| 2026 NIBRS offenses | 20,854,084 |
| Operating budgets | 63,825,383 |

These total approximately 191 MB of raw CSV, excluding historical files, companion tables, database indexes, and documents. They are not vector-store sizing estimates. The portal's displayed sizes can differ from the current object responses. [Verification details and sample hashes](san-diego-source-verification.json) record what was checked. A successful header sample does not establish full-file parseability, row counts, or statistical completeness.

The RIPA, calls, NIBRS, and operating-budget landing pages linked the [ODC Public Domain Dedication and License](https://opendefinition.org/licenses/odc-pddl/) when checked. This finding applies to those portal datasets; it does not assign that license to every City PDF, SanGIS layer, or third-party attachment.

## Results from the requested research platforms

### GitHub: valuable local work, with uneven readiness

The Union-Tribune's crime/calls/stops repository contains historical data and analysis for January 2019 through June 2021. It is especially useful for understanding a reproducible local investigation and checking preprocessing decisions. Its repository displays CC0-1.0, while the project also asks for attribution when using its findings. Keep the upstream source and journalist-derived processing identifiable. [SDUT crime/calls/stops](https://github.com/sdut-datadesk/crime-calls-stops-sdpd).

The separate 2019 veil-of-darkness project is relevant methodology; its license was not established by this review. Catalyst California's `sdpillars` explains local analytical choices, but expressly does not include the data tables and relies on a PostgreSQL source. Neither should be presented as a current ready-to-import official dataset. [SDUT veil of darkness](https://github.com/sdut-datadesk/veil-of-darkness-sdpd), [Catalyst sdpillars](https://github.com/catalystcalifornia/sdpillars).

OpenPoliceData offers a useful discovery registry and ingestion code. Evaluate its relevant adapters when adding an agency, while preserving original publisher URLs and dataset-specific terms. A registry entry is a lead, not evidence that an acquisition is complete. [OpenPoliceData](https://github.com/openpolicedata/openpolicedata), [dataset registry](https://github.com/openpolicedata/opd-data).

### arXiv: improve retrieval and evaluation

LegalBench-RAG is a particularly good fit for testing evidence retrieval. Its 2024 paper describes 6,858 query-answer pairs and precise supporting text spans; the repository documents character-range ground truth. Use the span format to design an equivalent San Diego evaluation. The repository's MIT license covers code; it explicitly directs users to the terms of ContractNLI, CUAD, MAUD, and PrivacyQA for underlying data. [LegalBench-RAG paper](https://arxiv.org/abs/2408.10343), [repository and data instructions](https://github.com/ZeroEntropy-AI/legalbenchrag).

MLEB offers multiple legal retrieval tasks and code for comparing embedding models. It is useful once the app is ready to choose a semantic retriever. Its published rankings should be reproduced on a local held-out corpus: the benchmark is maintained by a model vendor, and its jurisdictions and tasks extend beyond this application. Code and constituent dataset licenses are separate. [MLEB paper](https://arxiv.org/abs/2510.19365), [evaluation repository](https://github.com/isaacus-dev/mleb).

The 2026 LaborBench paper concerns statutory surveys in unemployment law. Its relevance is evaluation design: retrieval and reasoning should be assessed together, including missed authority and incomplete answers. It does not provide San Diego oversight evidence. [Benchmarking Legal RAG](https://arxiv.org/abs/2603.03300).

DocLayNet supplies annotated document layouts, including government and legal material, under CDLA-Permissive-1.0. Use it to evaluate reading order and table handling if local PDFs expose those failures. It belongs in an extraction test suite, rather than the application's factual corpus. [Official DocLayNet repository](https://github.com/DS4SD/DocLayNet), [paper](https://arxiv.org/abs/2206.01062).

### Zenodo: inspect the files behind the description

The most relevant downloadable dataset found in this search was ACTLAS, a European transparency and anti-corruption initiative catalog. Its record lists a CSV, codebook, and report, with collection spanning May 2024 to April 2026. It could inform a comparative governance taxonomy, but supplies no demonstrated San Diego operational evidence. The rendered record did not expose a usable license value, so reuse remains unresolved. [ACTLAS record](https://zenodo.org/records/20661380).

Two apparently promising legal-data hits are unsuitable as acquisition seeds. The MultiLegalPile record inspected contains a poster PDF, rather than the large corpus described in its abstract. The Swiss legal criticality/retrieval record contains a thesis PDF, rather than the described datasets. Follow their underlying projects only if a later retrieval experiment needs them. [MultiLegalPile poster record](https://zenodo.org/records/12837276), [Swiss thesis record](https://zenodo.org/records/8139264).

Direct Zenodo API/search access returned a 403 unusual-traffic response in this environment. Search-indexed primary records and file manifests were still reviewable. No access restriction was bypassed, and bulk downloads were not verified. The conclusion is that this search found no high-priority, verified San Diego corpus on Zenodo—not that none exists.

### Wolfram: optional entity context

Wolfram documents city entities and geographic/demographic properties, including ways to request source and date annotations. That could help normalize a city reference if a Wolfram integration becomes useful. It does not establish a freely redistributable local records collection. No Wolfram runtime or API query was executed, and bulk reuse rights were not verified. [CityData documentation](https://reference.wolfram.com/language/ref/CityData.html), [City entity documentation](https://reference.wolfram.com/language/ref/entity/City.html).

The repository search also found a concrete crime dataset, NYC Major Felony Incidents 2015, whose geography makes it unsuitable for San Diego evidence. Prefer direct Census and SanGIS references before adding a new service dependency for city context. [Wolfram NYC dataset](https://datarepository.wolframcloud.com/resources/NYC-Major-Felony-Incidents-2015).

## Other repositories: useful findings and important exclusions

LocalView deserved a deeper check because a national local-government transcript archive could save substantial collection work. Harvard Dataverse version 15 exposes CC0-1.0 metadata and transcript archives. I downloaded only the 35,200,750-byte metadata Parquet file and queried it with DuckDB. [LocalView dataset](https://doi.org/10.7910/DVN/NJTBEM).

The metadata contains 301,003 rows, including 35,053 with California-coded identifiers. Searching the place-name field for San Diego and the observed/predicted identifiers for the City code returned zero candidate rows. This does not prove no recording mentions San Diego, or that another release lacks City meetings. It does mean the examined release cannot currently be promised as a San Diego archive. The approximately 5.29 GB of transcript archives were not downloaded. The exact query, version, file IDs, checksums, and result are in the [verification artifact](san-diego-source-verification.json).

MeetingBank contains six municipalities: Seattle, King County, Denver, Boston, Alameda, and Long Beach. None is San Diego. Its author-hosted license page specifies CC BY-NC-ND 4.0, so proposed commercial use or redistribution needs a use-specific rights review. Its utility is meeting segmentation and summarization evaluation. The homepage's statement about future Zenodo hosting is not a verified Zenodo download endpoint. [MeetingBank dataset](https://meetingbank.github.io/dataset/), [license](https://meetingbank.github.io/license/), [paper](https://arxiv.org/abs/2305.17529).

Stanford Open Policing lists 382,844 San Diego stop records spanning January 2014 through March 2017, under its stated Open Data Commons Attribution license. This is a credible historical extension before RIPA, provided the old and new collection definitions remain distinct. [Stanford data catalog](https://openpolicing.stanford.edu/data/).

State reference material also merits a narrow collection. The 2026 California RIPA report concerns 2024 stops; publication year is not observation year. POST actions can supply official certification status, with careful distinction among status types. Current statutory text and relevant CourtListener opinions can support authority lookup, while RECAP docket coverage is incomplete. [California RIPA reports](https://oag.ca.gov/ab953/board/reports), [POST certification actions](https://post.ca.gov/Peace-Officer-Certification-Actions), [Penal Code 832.7](https://leginfo.legislature.ca.gov/faces/codes_displaySection.xhtml?lawCode=PEN&sectionNum=832.7.), [CourtListener API documentation](https://wiki.free.law/c/courtlistener/help/api).

EFF's Atlas of Surveillance provides source-linked leads about regional technology use, but its San Diego results include multiple agencies. Mapping Police Violence is another optional corroboration source; its methodology changed in 2026. Neither should silently replace an official local record or be assumed complete. [EFF San Diego search](https://atlasofsurveillance.org/search?location=San+Diego%2C+CA), [Mapping Police Violence methodology](https://mappingpoliceviolence.org/methodology).

## How to integrate the additions

This is a proposed design, not an implemented change.

Keep four storage roles explicit. Original documents and extracted passages supply citable evidence. Typed tables supply counts and comparisons. Reference tables supply aliases, jurisdiction, dates, and definitions. Evaluation datasets remain isolated from production retrieval.

Each acquired item should carry a source URL, publisher, jurisdiction, source identifier, retrieval time, content hash, document type, available license information, and parser version. Preserve publication date, observation period, effective date, and fiscal year as separate fields when applicable. Missing dates should remain missing, rather than being inferred from a download timestamp.

For documents, retain page and character coordinates, section headings, and links to attachments. Mark OCR-derived text and preserve the page image for inspection. Table captions, headers, and notes should stay with their table. The current character-based segmentation needs an extraction-quality review before assuming it will preserve legal sections or complex PDF reading order.

For CSV imports, save a source snapshot manifest and field mapping. Validate types, candidate keys, missingness, date ranges, category dictionaries, and join multiplicity. Derived results should record the query or transformation, input version, counting unit, exclusions, and denominator. An embedding may help find a dictionary definition; the actual sum or count should come from a validated query.

For entity resolution, keep SDPD, the County Sheriff, university police, and nearby municipal departments distinct. Link documents using agency IDs, contract numbers, report numbers, case identifiers, and explicitly verified aliases where available. Name similarity and textual co-occurrence should remain tentative leads.

When semantic retrieval is implemented, compare a hybrid of exact-term search and embeddings against the existing retrieval baseline. Apply jurisdiction and date filters before presenting evidence; favor exact identifiers for contract and policy lookups. Keep model name, version, chunking version, and content hash with each embedding so the projection can be rebuilt.

## Demonstrating improvement

Create a human-reviewed set of 120 San Diego questions before broad ingestion. A suggested allocation is 25 policy/version questions, 20 surveillance/procurement questions, 20 oversight/follow-up questions, 25 statistical questions, 15 jurisdiction/entity questions, and 15 deliberately unanswerable or incomplete-coverage questions.

Examples include locating the source that establishes an effective policy, comparing an audit recommendation with later follow-up, retrieving an award by contract number, calculating distinct stopped-person records for a specified release, and declining to state that an unavailable policy version governed a historical incident. These are task designs, not claims that every answer is already present.

Annotate required documents and supporting spans, acceptable answer variants, relevant dates, and the expected counting unit. Split by document family and time period so a near-duplicate or amended version does not leak across development and held-out sets. Keep public benchmark questions outside the production index.

Use a staged comparison:

1. Measure current retrieval on a frozen corpus and question set.
2. Add the selected local sources while holding retrieval constant.
3. Compare retrieval changes on that same expanded corpus.
4. Separately measure extraction, table-query accuracy, and citation rendering.

This separates gains from coverage from gains due to a different retriever. Report evidence recall at a fixed result budget, citation support, wrong-jurisdiction results, outdated-version errors, numeric correctness, appropriate abstention, latency, and local resource use.

Proposed acceptance gates are at least 90% recall of required evidence within ten results for answerable document questions, at least 95% correctly supported citations, and exact agreement on the reviewed statistical fixtures. Every deliberately wrong-agency fixture should be rejected. These are proposed targets, not measured application performance; publish both the sample size and the remaining failures.

## A bounded first implementation

Begin with a curated ALPR/surveillance packet, related contracts, applicable legislative records, and one audit/follow-up chain. A planning cap of roughly 100–300 documents is enough to expose linking and extraction failures without pretending to cover the whole City archive. The cap is a collection budget, not a verified inventory count.

Next, implement RIPA as its own typed import with dictionaries and companion tables. Add calls, NIBRS, and budgets only after the same snapshot, counting-unit, and validation conventions work. Geography should accompany any geographic analysis, and Census should accompany population comparisons.

Finally, expand CPP and Council history, add selected disclosure PDFs and DA letters, and evaluate the held-out questions. Defer large general legal corpora, bulk meeting transcripts without verified local coverage, and optional third-party services until a measured failure justifies them.

The research artifacts make this work reviewable: the catalog records each source's role, access check, rights status, and caveat; the verification JSON records bounded technical observations. No production ingestion or application behavior changed during this research.

## Source register

The CSV is the complete structured bibliography, including download and related URLs. This compact index maps its identifiers to the primary pages reviewed:

- SD01–02: [PAB reports](https://www.sandiego.gov/pab/reports); [policies](https://www.sandiego.gov/pab/policies).
- SD03–04: [Auditor reports](https://www.sandiego.gov/auditor/reports); [follow-up](https://www.sandiego.gov/auditor/reports/recommendation-follow-dashboard).
- SD05–07: [FY2026 awards](https://www.sandiego.gov/purchasing/bids-contracts/contracts2026); [FY2027 awards](https://www.sandiego.gov/purchasing/bids-contracts/contracts2027); [construction](https://www.sandiego.gov/cip/reports/constructioncontractsfy26).
- SD08–11: [Official documents](https://www.sandiego.gov/city-clerk/officialdocs); [resolutions](https://www.sandiego.gov/city-clerk/officialdocs/council-resolutions-ordinances); [Municipal Code](https://www.sandiego.gov/city-clerk/officialdocs/municipal-code); [Council reports](https://www.sandiego.gov/city-clerk/officialdocs/council-reports).
- SD12–14: [SDPD disclosures](https://www.sandiego.gov/police/data-transparency/mandated-disclosures/sb16-sb1421-ab748); [CPP reports](https://www.sandiego.gov/cpp/reports); [SDPD policies](https://www.sandiego.gov/police/data-transparency/policies-procedures).
- SD15–20: [RIPA](https://data.sandiego.gov/datasets/police-ripa-stops/); [calls](https://data.sandiego.gov/datasets/police-calls-for-service/); [NIBRS](https://data.sandiego.gov/datasets/police-nibrs/); [budgets](https://data.sandiego.gov/datasets/operating-budget/); [beats](https://data.sandiego.gov/datasets/gis-police-beats/); [ACS](https://www.census.gov/data/developers/data-sets/acs-5year.html).
- SD21–25: [DOJ RIPA](https://oag.ca.gov/ab953/board/reports); [POST](https://post.ca.gov/Peace-Officer-Certification-Actions); [California statutes](https://leginfo.legislature.ca.gov/faces/codes.xhtml); [CourtListener](https://wiki.free.law/c/courtlistener/help/api); [Get It Done](https://data.sandiego.gov/datasets/get-it-done-reports/).
- SD26–30: [Stanford](https://openpolicing.stanford.edu/data/); [SDUT data](https://github.com/sdut-datadesk/crime-calls-stops-sdpd); [SDUT veil of darkness](https://github.com/sdut-datadesk/veil-of-darkness-sdpd); [Catalyst](https://github.com/catalystcalifornia/sdpillars); [OpenPoliceData](https://github.com/openpolicedata/openpolicedata).
- SD31–35: [EFF](https://atlasofsurveillance.org/search?location=San+Diego%2C+CA); [Mapping Police Violence](https://mappingpoliceviolence.org/methodology); [CPE report](https://www.sandiego.gov/police/data-transparency/center-policing-equity-report); [LocalView](https://doi.org/10.7910/DVN/NJTBEM); [MeetingBank](https://meetingbank.github.io/dataset/).
- SD36–39: [LegalBench-RAG](https://arxiv.org/abs/2408.10343); [MLEB](https://arxiv.org/abs/2510.19365); [LaborBench](https://arxiv.org/abs/2603.03300); [DocLayNet](https://github.com/DS4SD/DocLayNet).
- SD40–42: [ACTLAS](https://zenodo.org/records/20661380); [MultiLegalPile poster](https://zenodo.org/records/12837276); [Swiss thesis](https://zenodo.org/records/8139264).
- SD43–44: [Wolfram CityData](https://reference.wolfram.com/language/ref/CityData.html); [Wolfram NYC dataset](https://datarepository.wolframcloud.com/resources/NYC-Major-Felony-Incidents-2015).
- SD45–46: [DA reviews](https://www.sdcda.org/office/officer-involved-shootings); [Sheriff transparency](https://www.sdsheriff.gov/resources/transparency-reports).
