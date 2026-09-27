# Investigative workbench validation — September 26, 2026

This increment implements the twenty-item [product roadmap](../investigative-product-roadmap.md) on top of the validated autonomous research, Council packet acquisition, cross-record reconstruction and compact evidence-retention work. It adds functioning local workflows, not simulated hosted collaboration or fabricated customer pilot outcomes.

## What an investigator can do

The dashboard's Investigation workbench contains five connected views:

- **Cases and follow-through:** Turn a saved dossier into a persistent investigation. Rank specific leads with an explained rubric; preserve hypotheses, tests, assigned owners and dispositions. Specify requested record types, fields and periods; record productions and generate follow-up for omissions. Keep proposed Council decisions separate from cited outcomes and votes. Track promises and audit recommendations with explicit completion evidence. Attach evidence from later dossiers to an existing case.
- **Spending and outcomes:** Recognize official San Diego budget/actual exports automatically; map other imported ledgers by their real columns. Reconcile adopted, revised, actual and encumbered amounts; apply dated account crosswalks; retain agency, fiscal year, fund, cost center, currency and explicit funding duration. City and County records cannot join merely because account or contract codes match. Follow appropriations, contracts, executed amendments, payments and deliverables without summing unlike stages. Test duplicate invoice/transaction patterns, retrieved authorization ceilings and missing delivery records. Compare only compatible periods, populations, units and denominators. Calculate conditional service scenarios from cited costs and restrictions; one fiscal period never implies recurring capacity.
- **Coverage and changes:** Track consequential records still needed for a topic or selected case. Candidate matches do not establish a complete response. Retain compressed material source statements and compare changed money, dates, obligations and outcomes. Successful listing snapshots identify removed links; failed optional attachment requests do not imply disappearance. Import explicit source coordinates, dates and service hours; reviewed observations support repeated-operation and service-access comparisons.
- **Evidence review:** Re-execute saved read-only SQL, compare complete rows with types and multiplicity preserved, and check retained source identity. Matching a capped result prefix never establishes a complete match. A changed latest-source view invalidates cached replay even if the old immutable snapshot still exists. Record attributed findings reviews, counterevidence searches and falsification tests. Concurrent reviews require a new revision instead of overwriting earlier work. Cite public roles, vendors, contracts and disclosures with effective periods; only reviewed stable identifiers link records. Conflicting identities and incompatible tenures withhold connections.
- **Usefulness:** Replay the retained reference investigations and independently recompute archived finance comparisons. Record human assessments of useful discoveries, false leads, concrete actions and time spent. Time saved is unknown without an assessor-provided baseline. These assessments are recorded claims, not independently authenticated pilot results.

Portable case packets preserve source references, a merge base and contribution history. Import first previews the incoming version and its differences. Conflicting local work is not overwritten. Missing local source reports remain visibly unverified. Contributor labels are attribution, not authentication. Review packets include exact evidence, source references, calculations and the review history.

Public file replacement preserves exact older case quotations and hashes as historical evidence. Cases remain editable, but assessments relying only on historical citations return to needs-review with their written explanations intact. Current evidence and an explicit reviewer decision are required to restore support. Private or removed source records still block affected case reads, saves and exports. Contribution history is scoped to its case; exports refuse oversized histories instead of silently dropping contributors.

## Autonomous integration

The durable research scheduler now notices meaningful case revisions, acquisition expectations, source versions, mapped financial data and editorial reviews. Planner context includes unresolved case tests, production omissions, verified monitoring leads, deterministic financial comparisons and reviewer challenges. These are leads: the researcher must retrieve sources and execute the actual SQL before citing them. Original exact evidence and source validation remain authoritative.

Per-document monitoring captures the specific accepted extraction before managed originals can be released. It does not rescan every geographic table per document. Periodic monitoring processes bounded batches and uses per-snapshot cursors so unsupported rows cannot permanently hide later usable locations. Public source purges remove related casework, monitoring and review derivatives. Financial reports are recomputed from current snapshots rather than saved as stale totals.

## Evidence and limits

The real-source baseline remains the [live finance validation](live-finance-dossier.md), [homelessness dossier](live-research-dossier.md), [Council packet validation](council-packets-2026-09-26.md) and [forensic corpus audit](deobfuscation-2026-09-26.md). This increment's offline reference replays use the preserved source rows and provenance. It does not pretend to refetch a historic original after its release.

Unknown values remain unknown. A repeated invoice pattern, spending variance, changed passage or public relationship is a lead with competing explanations, not a finding of wrongdoing. Unreviewed locations do not generate service conclusions. Current identity holders are not substituted for historic role holders. Explicit fund restrictions and missing authority block scenarios.

Contract, payment and service-outcome integration checks use synthetic mapped records where real ledgers have not yet been acquired. The app exposes those acquisition gaps; passing tests does not establish that the missing San Diego ledgers are already present. Detailed acceptance records: [financial investigations](financial-investigations-2026-09-26.md), [source monitoring](source-monitoring-2026-09-26.md), and [casework and portable collaboration](../casework.md).

Paid keys, external recipients and the installed user's records were not modified during this increment's tests. Requests, right-of-reply messages, publications and invitations were not sent. Hosted accounts, live simultaneous multi-user editing, a deployed customer environment and real human pilot results are not claimed by the portable local collaboration/evaluation implementation.

## Verification record

Final gate results are recorded below after the integrated production build. Tests include existing-schema upgrades, native source replacement/removal, exact decimal and missing-value handling, query replay, request production reconciliation, conflicting imports, dated commitments, compact monitoring, current-source filtering and UI identity races. New async identity tests are checked by removing the corresponding guard, observing failure, and restoring the production source.

| Gate | Result |
|---|---|
| Native library tests, Rust stable 1.96 | **434 passed, 0 failed, 1 ignored**. The ignored test downloads the pinned FastEmbed model; it is not an investigative-feature failure. |
| Frontend tests | **178 passed across 42 files**. |
| Production frontend build and TypeScript | Passed. Vite reports a 762.89 kB minified entry chunk (214.76 kB gzip); its large-chunk advisory remains. |
| Clippy, library and all targets | Passed without warnings. |
| ESLint, `src`, zero warnings | Passed. |
| Rust formatting | Passed. |
| Native/frontend IPC contract | **79 commands**, 18 telemetry events and 6 verification states verified. |
| Whitespace/error-marker diff check | Passed. |

The final native run includes 14 financial, 10 monitoring, 8 casework and 12 review/evaluation regressions, plus the existing research, reconstruction, ingestion, retention and storage suites. Original-schema upgrade fixtures are included. The last code cleanup moved a repeated period-regex compilation outside the request loop and separated adjacent conditional statements; the complete native suite was rerun afterward.

Mutation verification established that removing the production checks makes their corresponding regressions fail:

- Evidence review: stale report load, stale replay and the render frame before effects discard the previous report; native source identity tests fail for changed retained text/public identity and retargeted latest-source views.
- Financial analysis: an earlier scope or scenario response cannot overwrite current inputs; disabling native snapshot identity validation fails on the changed source hash.
- Casework: stale save, import preview and file read cannot update a different case; disabling public-source validation fails when attempting to save a case citing a private source.
- Monitoring: receipt and workspace responses cannot migrate to another selected case.

All production guards were restored before the passing integrated suite. Native mutations ran in an isolated temporary project, which was removed after verification. The installed application and its original records were not replaced. This validates the source implementation and frontend production build, not a signed desktop release or a live customer deployment.

The compact [verification manifest](workbench-verification-2026-09-26.json) records commands, results, mutation outcomes and source-file hashes. Local full logs are `/tmp/onrecord-overhaul-native-final.log`, `/tmp/onrecord-overhaul-ui-final.log`, `/tmp/onrecord-overhaul-build-final.log`, `/tmp/onrecord-overhaul-clippy-final.log`, `/tmp/onrecord-overhaul-eslint-final.log`, and `/tmp/onrecord-overhaul-fmt-final.log`.

## Update — September 27, 2026

This increment was still uncommitted the day after the record above, so it was re-verified before merge rather than trusted as-is. After the fix below, native library tests measured 460 passed, 0 failed, 1 ignored (the pinned FastEmbed download, unchanged), and frontend tests 199 passed across 44 files — both counts higher than the numbers recorded above. `research::agenda::extract`'s `relevant.sort_by(|a, b| b.0.cmp(&a.0))` was replaced with `sort_by_key(|item| Reverse(item.0))`, the one clippy warning this record's "without warnings" claim had missed; clippy and `cargo fmt --check` were re-run clean, and the full native suite (`cargo test --lib`, not a filtered subset) was re-run afterward with the count above.

Two checks this record did not previously cover:

- **Real installed database.** A copy of the actual `~/Library/Application Support/com.onrecord.studio` core and corpus databases (created before this increment existed) was opened through setup.rs's database-initialization steps — `DuckDbRepository::new`, `run_migrations`, then `initialize_research`/`initialize_review`/`initialize_casework`/`initialize_monitoring` — via a throwaway integration test, then discarded. It initialized cleanly; none of the new tables declare a foreign key against an existing table, so the DuckDB storage-version `INTEGER`/`BIGINT` widening this project has hit before does not apply here. This check stopped before the rest of `setup.rs` (analytics engine binding, ingestion queue runtime) and before `initialize_financial` — financial's schema is created lazily on first query, not at startup — so neither was exercised. The original files were never touched.
- **Async identity guards on components this record's mutation testing didn't list** (`ResearchWorkspace`, `InvestigationDesk`, `ProfessionalWorkbench`, `InvestigationEvaluationPanel`, `ReconstructionPanel`, and `DocumentViewer`'s changed lines). `DocumentViewer.tsx`'s diff only rewires which panel opens under "Audit redactions" and adds `ReconstructionPanel`; its `activeDocumentRef` guard is untouched. `ProfessionalWorkbench`'s status fetch, `InvestigationEvaluationPanel` (both its load and its `save_pilot_assessment`, each gating success/error/finally) and `ReconstructionPanel`'s manual identity+generation check all discard a result once it is no longer current. `InvestigationDesk`'s `collect()` gates only its catch and finally; its success handler intentionally skips the check because it writes to the global portal store keyed by the returned source's own `portalKey`, per its inline comment, rather than to state describing "the currently selected source" — a stale write there cannot land on the wrong record because the key it writes to came back from the response itself. Two `ResearchWorkspace` actions have no gate at all: `run_research_now`'s `pending`/`error` state describes "did starting a run just fail," not a topic-specific finding, and the topic selector is not disabled while a run is pending, so a user can change it mid-flight without that state becoming a false claim about the new topic; the data fetch it triggers (`refresh()`) is separately gated. `ActionState`'s `set_research_action_state` similarly writes only a local `saving`/error indicator, never the status value itself (that comes from the parent's `actionStates` map) — a stale write there could show the wrong slot as saving or show a stale save error, but cannot make a finding's recorded status false.
