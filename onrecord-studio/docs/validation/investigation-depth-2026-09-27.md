# Investigation depth and usability — September 27, 2026

The September 26 implementation added many workflows but did not establish that the default product produced valuable investigations with little manual work. The user identified shallow findings, inadequate automatic acquisition, too much configuration, and a confusing interface. This revision evaluates those failures using actual retained and newly acquired public records.

## What failed in the previous live investigation

The preserved [homelessness dossier](live-research-dossier.json) contains six failed deeper reads: Rust `usize` was bound as DuckDB `UBIGINT`, which `substr` does not accept. Eight follow-up searches returned no excerpts. Searches passed complex phrases and quotation marks into a literal substring search, even when their constituent terms existed in retained documents. The earlier live test checked that a dossier existed and had citations; it did not require those deeper reads to work.

The independent critic previously rewrote a draft against the same evidence. It could challenge wording but could not retrieve contradictory records or execute a new disproof query. A 64-item evidence ceiling could also silently exclude later discoveries after the first retrieval rounds filled the context.

These are product failures despite the earlier passing test suite. The historical artifacts are preserved rather than rewritten to erase them.

## Research behavior now

- Ranked retrieval understands quoted phrases and separated terms, accommodates PDF line wraps, ranks substantive matching passages, and favors distinct source files before a second passage from one file. It retains exact original character offsets and source text. The search remains bounded lexical retrieval; it does not claim exhaustive or semantic search.
- Deeper reads bind signed character offsets and check source hash, public visibility, extraction state and source lifecycle. Changed or removed sources do not silently become the old citation.
- Drafts undergo an empirical challenge: an independent model plan selects disconfirming searches, document reads and SQL comparisons, the native app executes them, and the editor sees their results. Missing evidence and failed tools are explicitly reported rather than described as corroboration.
- Retained evidence and model context are separate. Later results can enter the next context; draft-cited sources are prioritized. Whole optional context items can be omitted with an explicit coverage notice, without rewriting source quotes or query values. Required draft evidence that cannot fit causes a clear failure.
- Seeds now include shelter performance, settlement recoveries, invoice timing, revised appropriations and implemented audit recommendations. These pursue specific consequences and competing explanations, rather than just broad subject mentions.
- A discovered official record can be acquired and searched during the same investigation before a records request is proposed. Successfully processed documents enter the refreshed search/catalog even when the remaining collection is unfinished. Discovery requires independently attested URLs; a model-proposed URL alone is not sufficient.
- Before/after/until fiscal-year conditions in supporting quotations must survive in the finding interpretation. Exact quotation matching alone did not catch a live draft that turned an explicitly future service transition into a possible current change.

## More substantive automatic acquisition

SDHC board reports often use extensionless PDF URLs. The acquisition adapter now recognizes literal board-table report links as downloads, preserves the surrounding agenda description, and prioritizes substantive reports over navigation and slides. The ordinary PDF parser still verifies the actual bytes.

Automatic seed collections include SDHC board agreements/performance packets, the City purchasing page, the FY2027 contracts register, and the FY2023 year-end financial report. Existing maintenance schedules refresh active registered sources; paused and removed sources remain under operator control.

These records distinguish proposed contract authority, executed agreements, payments, capacity and outcomes. A static HTML summary counter displaying zero is not treated as a trustworthy calculated total without checking its record rows.

## Financial depth and less manual work

The financial engine now looks for offsetting account movements, persistent gaps between growing adopted plans and actual execution, and rebounds that remain below an earlier baseline. Independent multi-year aggregate queries prevent a truncated account ranking from being mistaken for a complete department total. Every derived lead carries exact arithmetic, source rows, alternative explanations and the records that would settle it.

The real archived FY2023 homelessness account pair illustrates why this matters: a $10,563,304 negative adopted-to-actual difference in account 512059 is partially offset by $8,767,029 in account 512066 within the same cost center. The combined difference is $1,796,275. That is a candidate classification/reallocation explanation, not proof that a transfer occurred or that services were delivered. Journal entries and program/contract crosswalks are the next evidence to obtain.

The final acquisition audit found another direct impediment: automatic table imports unconditionally marked snapshots partial, which correctly caused the financial engine to withhold its most useful comparisons. The correction recognizes only the two exact official City operating-budget/actuals CSV exports, after hash-verified import, parser validation and an automatic-origin receipt. It also repairs eligible legacy automatic snapshots; manual partial snapshots, unknown URLs, pagination, changed hashes and rejected rows remain protected. “Complete” means the published file was imported completely, not that all City costs are represented or the fiscal year is finalized. Coverage changes wake the research scheduler.

## Default experience

The default dashboard is an investigation briefing across topics. It surfaces specific findings, why they matter, exact evidence, competing explanations and the next action. Full dossiers open on demand. Manual planning, source configuration, storage controls and response metrics are secondary tools rather than the first screen. Saved research failures remain visible. Loading, paused, unavailable credentials and failed research states are distinct.

Automatic scheduling can start from one useful artifact after a bounded initial collection window; a continuous stream of new records no longer resets that window indefinitely. Impossible saved model-call caps, depleted daily budgets and missing credentials have explicit recovery states. The displayed completion date comes from an actual saved dossier, rather than a failed attempt. Current provider/model identity is checked immediately before dispatch and after the response so a settings change cannot redirect new evidence to the old provider or publish a stale reply.

Automatic collection checks the **resolved** source identity and canonical URL under the same lock used for pause/remove actions. Aliases and redirects cannot re-create an existing source or overwrite its saved collection recipe. Explicit operator resumption remains available.

A bounded test exposed a real cancellation deadlock: pause/removal held the lifecycle lock while joining a finishing worker that was itself waiting for that lock. Retirement now observes cancellation while waiting, after task/parser writes have finished. The [negative control](collection-cancellation-negative-control-2026-09-27.txt) reproduces the old lock cycle and fails at the intended three-second timeout. This does not abort parsing or weaken source-lifecycle checks.

## First integrated run: failure preserved

The [first new live attempt](investigation-depth-first-attempt-2026-09-27.json) acquired ten supplied official records plus two independently discovered City action-plan PDFs. All 16 follow-up searches returned passages; all 12 deeper reads succeeded. The independent challenge added 24 evidence items and made three successful deeper reads with no failed tools. It retained 144 evidence items and produced two findings, six requests and seven further questions in seven model/search calls (787 seconds).

This was **not an accepted end-to-end result**. The meeting assertion failed: the researcher missed a relevant September 28 Council action even though it had acquired the agenda. The generic search passages ended at the start of an item's title. Two other findings failed exact quotation validation and were withheld. A human review also found that the surviving Day Center interpretation dropped the source's “after FY2027” effective date, that recommendation reports were described as completed awards in follow-up questions, and that request periods drifted beyond the dossier date. The first artifact remains unchanged so these failures cannot disappear into a rewritten validation report.

The corrections add complete agenda-item extraction and source-bound actions, same-original passage-reference correction without rewriting quotes, explicit effective-year validation, stronger source-status and request-period instructions, and diagnostic previews for rejected quotations. These checks have deliberately narrow claims: they improve source fidelity and prevent measured failure modes; they do not establish all model interpretations as true.

## Acceptance evidence

The [accepted live dossier](investigation-depth-live-2026-09-27.json) completed in 800.86 seconds using seven billed model/search calls. It contains four retained findings, six records-request drafts, seven follow-up research questions and three September 28 Council actions. The [live execution log](investigation-depth-live-2026-09-27.txt) preserves acquisition, call progression, original release and the passing integration test.

| Real-source check | Observed result |
|---|---|
| Acquisition | Ten supplied official records plus two independently discovered City PDFs acquired within the investigation; 23 new evidence items became available after acquisition. |
| Retrieval | All 16 follow-up searches returned passages; all 12 deeper reads succeeded. The retained dossier contains 150 evidence items. |
| Independent challenge | 22 additional evidence items/results, three successful deeper reads, no failed tools. This PDF/HTML investigation had no imported actuals ledger and executed no financial SQL. |
| Source fidelity | Exact quotes and dated agenda evidence validated. One proposed finding was withheld because its interpretation omitted the source's “after FY2027” condition. |
| Council follow-through | Native extraction recovered three complete relevant items. The model recommended Item 605; source-bound fallback supplied Items 604 and 606, explicitly recorded as deterministic suggestions. None implies adoption, spending or a comment deadline. |
| Storage | All 12 processed public originals were released; retained derivatives remained searchable and original links remained attached to the evidence. |

The retained findings are specific, reviewable leads:

- **Police spending scope:** compare the FY2026 report's Neighborhood Policing exclusion with the FY2027 narrative's $5.2 million Police homelessness outreach amount and its attachment's exclusion. Distinct Police units could explain the difference; the request seeks an internal scope crosswalk before treating the headline totals as comparable.
- **Projected savings composition:** reconcile a $507,141 prior-year SDHC overbilling credit, $95,536 shifted to a County grant, $441,143 associated with Rachel's Promise delays and $539,542 in storm relocation/repair costs offsetting savings. These are different components of a projected variance, not amounts to sum as waste or treat as final savings.
- **Encampment grant lineage:** connect the State Auditor's June 2023 Round 2 funding snapshot with the proposed $3,666,572 Riverbed amendment on September 28, 2026. Shared program names do not establish that these are the same dollars; the records request seeks the State–County–City award, agreement and payment chain.
- **Shelter contracts and outcomes:** reconcile proposed Imperial and Single Adults agreements with executed terms, actual payments and contract-specific housing exits. Aggregate reported outcomes cannot establish either contract's effectiveness.

All six request drafts seek existing records through September 27, 2026. No request was sent and no participation action was taken. The meeting recommendations remain source-bound suggestions whose latest agendas and participation instructions need checking when used.

The separate [financial replay](financial-depth-2026-09-27.md) verifies four deeper connections against archived official ledger rows. The [SDHC acquisition check](sdhc-acquisition-2026-09-27.md) starts with the board page and ordinary acquisition guidance, recovering substantive shelter reports without manually supplying their document URLs. These checks establish additional paths beyond the PDF dossier; neither is presented as an independent fresh financial audit.

### Regression gates and deliberate failure checks

The final gate passed: **460 native tests** (one ignored), **199 frontend tests** across 44 files, Rust formatting, TypeScript, ESLint, IPC contracts (79 commands, 18 events, six states) and the production frontend build. Clippy completed successfully with one non-blocking `unnecessary_sort_by` advisory; Vite reported its bundle-size advisory. [Gate output excerpts](investigation-depth-checks-2026-09-27.txt) record the completed checks.

The [accepted-dossier browser review](investigation-depth-briefing-browser-2026-09-27.json) records the exact input SHA-256 and verifies the visible quotation, original link, saved timestamp and selected request draft. The request action is visible within the first viewport on both desktop and mobile; neither layout has horizontal overflow or runtime errors. The [desktop](investigation-depth-briefing-browser-2026-09-27-desktop.png), [mobile](investigation-depth-briefing-browser-2026-09-27-mobile.png) and [request](investigation-depth-briefing-browser-2026-09-27-request.png) screenshots were visually inspected. [Reproduction instructions](briefing-browser-reproduction.md) distinguish this component review from the separate native live investigation.

[Identity negative controls](identity-guard-negative-controls-2026-09-27.txt) demonstrate failures when source-hash, provider/model or automatic collection lifecycle guards are removed. The [cancellation negative control](collection-cancellation-negative-control-2026-09-27.txt) reproduces the former lifecycle-lock deadlock. Financial scope changes, stale UI responses and removed dossier snapshots also have mutation-proven regression checks. Guard removals were confined to isolated copies; the final workspace and native test executable contain restored guards.

### Limits of the evidence

The accepted run used `openrouter / openai/gpt-5.4` and reported 233,394 input tokens and 81,669 output tokens across its calls. This is a substantial investigation, not an instant or free operation. Token totals are provider-reported usage, not a dollar-cost estimate. Saved per-run and daily call limits still apply.

Ten initial URLs in the integration test were supplied by the test; only the two later PDFs were autonomously discovered in that run. Ordinary board-page acquisition is separately exercised above. The live test used an isolated native database, while browser review renders production components with a read-only status fixture. It does not establish a packaged desktop end-to-end session or unattended operation over weeks.

The investigation has no AP ledger, executed shelter agreements or contract-level outcome dataset. It cannot establish waste, ineffective shelters, an improper Police allocation or whether historical grant balances are the dollars in the current amendment. Exact citations establish source fidelity; the interpretations and practical usefulness still require editorial review. The default briefing exposes these limitations with the findings and next tests.
