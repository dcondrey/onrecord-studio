# San Diego investigations

The primary product is an investigation workspace for homelessness spending and outcomes, encampment abatement, access to public bathrooms, police budget priorities, and contract performance. City, County, Housing Commission, and regional records have different jurisdictions and measures; the research engine must preserve those distinctions.

## What runs autonomously

The native desktop service runs while the app is open, independently of the selected screen. It acquires bounded primary-source collections, imports eligible tables, maintains storage, and rotates through five research topics. Each case responds to new searchable evidence after a 60-second settling period and a 10-minute cooldown, subject to the shared daily call budget. A case with unchanged evidence falls back to a six-hour interval; failed attempts wait one hour before retrying. Defaults are four retrieval/comparison passes, `gpt-5.4`, and 24 model calls per UTC day. On first setup, OpenRouter is selected if its key is present; otherwise direct OpenAI is selected. Change these in **Research settings**. The selected provider reads only `OPENROUTER_API_KEY` or `OPENAI_API_KEY` from the process environment or `~/.env`; selected public-record excerpts and query results are sent to that API. OpenRouter forwards them to its model provider. An installed Ollama model with structured JSON support can be used locally.

Initial acquisition covers Council agendas, homelessness program reports, IBA analyses, operating budgets and actuals, encampment abatement, the State homelessness audit, Get It Done, City audits, RTFH reports, SDHC budgets, and finance reference tables. Each acquisition is bounded to 4 City pages or 12 other pages, depth 2, and 150 resources. Sources already saved, paused, or removed are not recreated. Failed starts have persisted retry times. Activity records what actually downloaded and extracted; a registered source is not proof of complete coverage.

After a dossier, optional provider web search looks for up to six additional primary records that address its unresolved questions. Only URLs returned in the search tool's source list and belonging to the configured official publishers can enter the acquisition queue. A later investigation can analyze successfully acquired records. Search snippets are not treated as findings. Browser-dependent archives and failed extractions remain visible limitations.

## How a case develops

1. Retrieve several relevant passages per document, favoring costs, outcomes, contracts, and commitments. Preserve source links, file hashes, and character offsets. Provide exact analytical table schemas and snapshot metadata.
2. Generate questions from the evidence, pursue new phrase searches, read deeper passages, and execute restricted SQL. SQL cannot read arbitrary files or make network calls. Results use a single column dictionary and arrays of values. Nulls, partial coverage, rejected rows, and truncation remain explicit.
3. Repeat retrieval and comparisons for the configured number of passes. Carry earlier questions and the previous dossier into the investigation. Tool failures are visible to later passes so they can correct a query or narrow a claim.
4. Produce findings, alternative explanations, falsifiable next tests, tailored public-records request drafts, and relevant dated meeting actions. A separate model pass challenges the draft against the same sources.
5. Validate exact quote matches and evidence IDs before saving. Meeting dates need a matching full date quoted from an official source. Save a compressed dossier and display its evidence, limitations, and investigation trail.

Quotation validation establishes that text was retrieved; it does not establish that the model's interpretation is correct. The UI separates interpretations, counter-evidence, alternatives, and next tests. Cost comparisons must align fiscal period, fund, budget cycle, scope, and measure. A variance alone is not evidence of waste. Lack of a record in a bounded search does not prove the agency never collected it.

## Reproducible financial investigations

When both official City operating exports are imported, native SQL runs before the model: fiscal-year/cycle coverage, account-key reconciliation, department spending history, adopted-allocation changes, and Police overtime comparisons. These comparisons restrict the fund to General Fund 100000 and expense accounts beginning with 5; budget baselines must explicitly say adopted. Two-digit year codes are normalized, and historical spending comparisons use the latest common ended fiscal year. Empty budget-cycle labels are exposed rather than classified as adopted.

Each side is aggregated before joining by fiscal year, funds center, and account, preventing many-to-many multiplication. Department labels from both sources are retained. Missing counterparts stay null; differences are withheld for incomplete amounts. SQL evidence retains source snapshot URLs and hashes alongside the executed query. The model can then pursue the consequential differences, test explanations, and draft requests for revised authority, transfers, invoices, staffing, or deliverables. Original adopted-budget variance does not establish spending beyond revised legal authority.

## Following through

The home screen leads with findings and connections, specific request drafts, and meetings/actions. Each request explains the missing evidence, likely custodian, and question it would resolve. Requests are reviewable and downloadable; the app does not submit them. Actions have persisted to-do, in-progress, completed, and dismissed states. Prepare a case packet to review and export linked findings, cited rows with column labels, request drafts, meeting actions, source URLs/hashes, full retained evidence, SQL, snapshot identities, and current action states. Markdown is intended for editorial review; a versioned JSON bundle preserves the exact saved report for independent checking. A packet is a dated snapshot and can be refreshed after casework changes. Completing an action is not a verification of a claim. Meeting recommendations distinguish historical follow-through from upcoming meetings and link back to the source. No agenda date is inferred from a budget year or routine schedule.

The manual investigation desk, broader source catalog, Evidence workspace, SQL workbench, and briefing builder remain available. New briefing templates focus on spending, services, and verifiable public follow-up. Existing customized briefings are preserved.

## Storage

On the first installation of this research feature, managed public downloads switch to reference-only retention, including eligible previously processed downloads. Later user retention choices are preserved. Storage upkeep releases originals only after successful text extraction with a matching projection hash, or a complete accepted tabular import with its row ledger. It retains original URLs, raw-file hashes, derived searchable text/tables, and durable release receipts. A receipt and recovery path cover interrupted deletions.

Local imports, unavailable or unstable source URLs, failed extraction, and incomplete tables retain their originals. This is deliberate: a link cannot reconstruct an incompletely processed or private source. The publisher can change or remove a linked file, so reference-only mode cannot guarantee future recovery of its original bytes.

Derived text and tables live in DuckDB; dossiers are explicitly Zstandard-compressed. Model context uses selected passages and compact tabular results instead of entire downloaded files. Searchable text, row ledgers, and snapshot history still consume storage. Lossy summaries do not replace canonical evidence.

## Verification

The normal quality gate is in the repository's `CLAUDE.md`. Research tests cover bounded read-only reconciliation, absent versus zero counterparts, quote/date rejection, durable call limits, retention migration, compressed dossier round trips, source retry behavior, and UI follow-through. The stale polling regression test is checked by temporarily removing its guard and observing failure.

The opt-in live integration uses temporary databases, downloads published San Diego homelessness funding analyses and the State audit, runs four billed model calls, and checks acquisition, extraction, cross-record findings, request generation, original release, and continued search:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test research_live -- --ignored --nocapture
```

The larger finance check acquires both complete City exports through the native queue, automatically imports their tables, runs the reproducible comparisons and investigation, and verifies original-file release:

```sh
cargo test --manifest-path src-tauri/Cargo.toml --test research_finance_live -- --ignored --nocapture
```

The live checks do not alter installed application data. It tests sampled records, not complete archive coverage. Public websites and model outputs change, so it remains separate from deterministic regression tests.
