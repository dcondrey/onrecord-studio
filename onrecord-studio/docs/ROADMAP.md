# OnRecord Studio roadmap and handoff ledger

> Renamed from `docs/TODO.md`, which matched a `todo.md` pattern in a contributor's
> personal global gitignore and was therefore invisible to every clone.

This file is intentionally explicit so work can resume safely after a rate
limit, context compaction, or a change of contributor. Search for `TODO(` and
`NOTE(` in `src/` for the code-adjacent decisions.

## Session log

### Continuation — 2026-08-27 release-hardening pass

Current Rust verification is green after the changes below: `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace`
(223 unit tests passing, 0 failing, plus doctests). The frontend verification is also green:
`npm run lint`, `npm run test:unit` (66 tests), `npm run build`,
and `npm run check:ipc` (30 commands, 18 telemetry events, 6 verification states).

- [x] Every embedded `.msg` descent now spends the shared container-depth budget. The parser
      exports only immediate attachments, and the durable worker schedules nested messages through
      the same bounded path as other containers. Embedded-message identity is determined from the
      exported long filename that format detection receives, and old frontier rows containing the
      removed `skip_msg_attachment_expansion` field still deserialize. A real nested CFB fixture
      verifies the boundary.
- [x] Structured JSON parsing and recursive discovery run on the blocking executor. The async
      worker receives only bounded scheduling summaries; malformed JSON remains a no-op. A focused
      regression covers ordinary links, ArcGIS companions and layers, pagination termination, and
      malformed input.
- [x] Durable retries are explicitly pinned to the existing `AppError::is_retryable` policy:
      network, rate-limiter, and portal-capacity failures retry up to the attempt ceiling; parsing,
      portal-security, storage, and internal failures terminate immediately.
- [x] Startup removes only the dedicated uncommitted `documents/.staging` tree. It refuses a
      staging symlink or non-directory and deliberately retains committed attachment/container
      trees because those paths are evidence used by viewing, provenance, pause, and retry flows.
      Tests cover cleanup, idempotence, committed-object preservation, and symlink refusal.
- [x] Re-audited two stale lower-severity claims. `Retry-After` is already capped at 120 seconds in
      `src-tauri/src/portal/client.rs`, and robots policy is fetched, cached, and enforced by the
      crawl host-policy path. Deterministic (unjittered) retry timing and non-persisted scheduler
      state remain separate lower-priority concerns.
- [x] Re-audited source-guidance boundaries. The frontend now enforces the backend's 100-rule and
      1,024-character per-rule limits before submission, and malformed NextRequest timestamps are
      treated as absent dates instead of being compared lexically. The valid document-carried date
      path remains NextRequest-only.
- [x] Consolidated latest-request sequencing in `useLatestRequestGate`. Search, corpus patterns and
      excerpts, provenance, legal findings, document lists/opening/history/recovery, brief exports,
      and source discovery/import flows all reject superseded results, errors, and `finally` state
      transitions. Closing a source dialog invalidates its native request even though the IPC call
      itself cannot yet be aborted. The hook has token, explicit-invalidation, and unmount tests;
      existing viewer tests continue to exercise late native responses across document changes.
- [x] Verification after the guidance and sequencing pass: frontend lint, TypeScript, production
      build/IPC contract, and 66 unit tests are green; Rust fmt, strict clippy, all 223 unit tests,
      and doctests are green.

### Continuation — 2026-08-27 evening

Current verification is green: `cargo fmt --all -- --check`,
`cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
(217 unit tests plus doctests), `npm run lint`, `npm run test:unit` (61 tests),
`npm run build`, and `npm run check:ipc` (30 commands, 18 telemetry events, 6 verification states).

- [x] Finished the interrupted source-scoping work. Only NextRequest's API-carried
      `DocumentMetadata.created_at` is eligible for document-date guidance; crawled, ArcGIS,
      local, pasted, and other synthesized metadata dates are ingestion instants and now
      contribute no date measurement. The backend validates real ordered calendar dates, and the
      guided-repair dialog now exposes filename, content, organization, size, and date rules with
      frontend validation. Regex rules are line-delimited so commas inside expressions survive.
- [x] Made post-extraction guidance safe across resumed syncs. A newly created unfinalized stub can
      be discarded, but a document with finalized FTS evidence cannot be deleted and orphan its
      corpus row. Both paths have repository regressions.
- [x] Failure is no longer rendered as a negative finding in evidence search or legal compliance.
      Failed searches retain and label the last successful result set; an empty failed query says
      unavailable, and a failed legal-findings query no longer says "No active findings recorded."
- [x] Saved-source initialization now distinguishes loading, genuinely empty, and unavailable.
      A failed `list_saved_portals` call no longer presents a returning user with first-run
      onboarding in either the overview or source sidebar.
- [x] Repeated runtime-poll failures are rate-limited in the activity log and recovery is recorded,
      preserving the earlier events an investigator needs to diagnose the outage.
- [x] Fixed the CPRA confidence formula's accidental executable doctest, which was the only failure
      after all 217 Rust unit tests had passed.

### Verification state at the end of this session

Green after the crawl-depth and provenance work, run together:
`cargo clippy --workspace --all-targets -D warnings` (which compiles the test code too),
`cargo test --workspace` (166 passing, 0 failing), and `node scripts/check-ipc-contract.mjs`
(27 commands, 17 telemetry events).
That 166 is not comparable to the 86 and 90 recorded earlier: a concurrent session was writing
`corpus.rs`, `sqlite.rs`, `c2pa.rs`, `investigation.rs`, `validation.rs`, `format.rs` and
`svg.rs` in the same tree, and its tests are in that total. `cargo fmt --all --check` fails on
four hunks, all in that session's in-flight code, none in the ingestion work described here.
Run cargo from `src-tauri/` or with `--manifest-path src-tauri/Cargo.toml`.

Not re-run for that change because it touched no frontend or release surface: `npm ci`,
`npx tsc --noEmit`, `npx eslint . --max-warnings 0`, `npm run build`, `cargo build --release`.
All five were green at the end of the preceding session, when the last frontend change landed.

### Completed

- [x] **Analytics read a launch-time snapshot.** `AnalyticsEngine` and `DuckDbWorkspace` were built
      at startup with a separate connection that `ATTACH`ed the core file. Such a connection is
      frozen at attach time and never sees a later write, so every agency profile and correlation
      was computed against the database as it stood at launch — empty on a fresh install. Both now
      take a live sibling connection (`DuckDbRepository::analytics_connection`, `try_clone`).
      Pinned by `analytics_nodes_observe_writes_made_after_they_were_bound`, which fails with
      `left: 0, right: 1` against the old wiring.
- [x] **Legacy SQLite core bricked every upgrade.** DuckDB cannot open a pre-migration SQLite file;
      startup died with an engine error. `storage/legacy_import.rs` classifies by header bytes,
      converts in one transaction in foreign-key dependency order, verifies row counts before
      commit, retains the original as `*.legacy-sqlite.bak`, and leaves the original untouched on
      any failure. 4 tests.
- [x] Detach-failure alias poisoning in both analytics nodes (was `BUG(analytics)`, duckdb.rs:191):
      a failed recovery detach left the `db` alias bound while state said otherwise, rejecting
      every later attach. The node is now rebuilt instead.
- [x] Removed 5 dead duplicate modules at `src-tauri/src/` (`compression_oracle`, `hypothesis`,
      `neural_inference`, `raster_audit`, `svg`) — untracked, undeclared in `lib.rs`, never
      compiled. Copies retained under the session scratchpad.
- [x] Release blockers: removed `-C target-cpu=native` (release binaries would SIGILL on any CPU
      older than the build machine); dropped `http://localhost:5173` from the release CSP; wrote
      `.github/workflows/ci.yml` (was 0 bytes) covering lint/tsc/IPC-contract/fmt/clippy/test/audit
      /bundle on three platforms; added `.cargo/audit.toml`; un-ignored both lockfiles and ignored
      `src-tauri/gen/schemas/`; removed the unreachable shell plugin from all three declarations.
- [x] Documentation drift: rewrote `docs/architecture.md` (was stale in nearly every specific —
      wrong engine, wrong queue capacity, a Safari User-Agent the client does not send, an FTS5
      layer that does not exist, and a three-command IPC sandbox that never existed); corrected 4
      false claims in `analysis_methods.md` and the malformed endpoints plus `per_page` bounds in
      `api_spec.md`.

- [x] **Site-crawl depth was silently not enforced on the download path, and the JSON chain had no
      cap of its own.** Not "unbounded", which is what the original note claimed and what an
      earlier draft of this entry repeated: `maximum_resources` defaults to 100,000,
      `maximum_pages` to 5,000, and every discovered download passes `reserve_resource`, so total
      work was always finite. What was true is that `IngestionTask::DownloadBinaryPayload` carried
      only `container_depth`, every construction site hardcoded it, and
      `expand_structured_document_links` restarted its derived links at a literal `1`, so
      `maximum_depth` governed nothing a JSON payload did.
      `DownloadBinaryPayload` and `CrawlArcGisLayer` now carry serde-defaulted `crawl_depth` and
      `structured_depth` (the durable frontier holds rows serialized before either field existed;
      those resume at 0, exactly as they behaved). The three axes travel together as
      `PayloadDepth { container, crawl, structured }` through `process_binary_payload`,
      `extract_document` and `expand_container`.
      **Three independent axes, deliberately not one budget.** `container` counts archive nesting
      against `MAXIMUM_CONTAINER_DEPTH`. `crawl` is site depth, identical in meaning to
      `CrawlPublicResource::depth`, against `CrawlPolicy::maximum_depth`. `structured` counts
      JSON → download → JSON hops against the new `MAXIMUM_STRUCTURED_CHAIN` (8, mirroring the
      container ceiling). An earlier attempt funded the chain out of `maximum_depth` and that was
      wrong: it meant a Socrata or ArcGIS catalog sitting eight pages into a portal silently
      stopped yielding documents, which is the opposite of what an evidence tool should do.
      Termination argument, worth auditing rather than re-deriving. Three edge kinds, three bounds:
      1. *Derivation* (a JSON payload yields a download or a layer that is JSON again) increments
         `structured`, so a pure derivation chain is capped by `MAXIMUM_STRUCTURED_CHAIN`.
      2. *Laundering* (JSON yields a crawl URL, that HTML page yields a download that is JSON)
         resets `structured` to 0, but every HTML hop enters through `process_public_resource` or
         `process_wordpress_google_drive`, both of which begin with `reserve_crawl_page` and
         therefore spend `crawl`. Laundering is capped by `maximum_depth`, so the combination is
         bounded by roughly `maximum_depth × (MAXIMUM_STRUCTURED_CHAIN + 1)`.
      3. *Pagination* (`arcgis_search_next_page`, `CrawlArcGisLayer` next offset) deliberately holds
         both `crawl` and `structured`, so neither ceiling bounds it. This is the one edge kind
         where a page can hand back a fresh next-page URL indefinitely. It is bounded only by
         `maximum_pages` (5,000) and `maximum_resources` (100,000), which is exactly how it behaved
         before this change, and `process_arcgis_layer` additionally dedups by page hash and stops
         at a 100M offset. Not a regression, but it is why the depth ceilings are defence in depth
         on an already-bounded system rather than the thing keeping the system finite.
      **ArcGIS layer pagination no longer spends site depth.** `reserve_crawl_page` was split:
      the depth check stayed, and the page budget plus rate limit moved to `reserve_paginated_page`,
      which `process_arcgis_layer` now calls. Walking one feature layer's offsets is not descending
      the site. This is a real loosening at the boundary, not a redundant check removed: a layer is
      discovered at its page's depth + 1, so one found at exactly `maximum_depth` used to be
      dropped. `maximum_pages`, the per-layer `arcgis_page_hashes` dedup and the 100M offset
      ceiling bound it.
      Wider than the original note: `process_public_resource` fed the page's own ArcGIS and Tableau
      companion URLs to `prepare_crawl_url` at a literal `1` too, the same defect as the
      link-derived companions below them. Every construction site was given an intentional value:
      portal-API documents and pasted content `(0, 0)`, ArcGIS feature pages the layer's own pair,
      container members and `.msg` attachments their parent's unchanged, since neither is a new
      chain hop.
      The serde defaults are 0 rather than the ceilings, deliberately: a resumed frontier row gets
      a fresh budget, matching `pages_started`, `resources_started` and `visited_urls` already
      being in-memory and resetting on restart. Defaulting to the ceiling would drop in-flight work
      without adding any bound.
      Pinned by `structured_expansion_is_bounded_by_the_chain_budget_not_by_crawl_depth`,
      `a_catalog_at_the_crawl_ceiling_still_expands` (the case the discarded design killed), and
      `a_frontier_row_written_before_the_depth_fields_existed_resumes_at_depth_zero`.
      **Unverified:** `WorkerContext` is constructed only inside the worker spawn and cannot be
      built in a test, so the full JSON → download → JSON path is not exercised end to end. The
      `may_expand_structured_links` predicate and the persistence round trip are; the wiring
      between them was read, not run.
      **Net behavior change versus the original code, one item:** crawl URLs derived from a JSON
      payload now cost site depth (`crawl + 1`) where they used to be handed a literal `1`. A
      catalog at the depth ceiling still yields its downloads; what it no longer does is spawn HTML
      crawls that ignore the policy. That is the defect being fixed, but it is the one place a
      portal could return fewer pages than before, so it is the first suspect for any
      "this portal stopped returning documents" report.
      `maximum_depth` defaults to 8 (`types/source.rs:18`) and `validate_crawl_policy` caps it at
      32, but no surface raises it: `commands/ingest.rs:50` accepts `Option<CrawlPolicy>` and both
      frontend call sites (`Header.tsx:90`, `:175`) hardcode `crawlPolicy: null`. Wiring that is
      the next step, not raising the default.
- [x] **Discovered documents were all stamped with the portal's URL as their origin.**
      `prepare_discovered_download` wrote `Discovered from {portal.canonical_url}` regardless of the
      page the link was on or the host the bytes came from — in an evidence tool, third-party bytes
      recorded as the portal's. It now takes the real referrer and calls
      `describe_discovery_origin`, which names the discovering page and, when the payload host
      differs from it, appends `; payload hosted on {host}`. The four call sites carry four
      truthful values: the requested URL for a crawl target that turned out to be a download (so a
      cross-host redirect is now visible), the containing page for a link, the source JSON's own
      `download_url` (falling back to `local document {id}` when there is none) for structured
      expansion, and the folder identity for WordPress/Google Drive listings. Nothing read the old
      string. Pinned by `discovery_origin_names_the_page_and_flags_a_cross_host_payload`.
- [x] **A 1,455-byte `.xlsx` killed the whole application.** calamine's `worksheet_range` builds a
      dense matrix spanning the sheet's bounding box before any limit can inspect it; a sheet with
      only A1 and XFD1048576 spans 17,179,869,184 cells and aborts on allocation — and an abort does
      not unwind, so `spawn_blocking` could not contain it. The extent is now measured by streaming
      the cell records (the declared `<dimension>` is advisory and was absent in the attack file).
      `rejects_a_sparse_sheet_whose_bounding_box_would_allocate_a_dense_matrix` builds the real
      attack file; it went from SIGKILL to pass.
- [x] **Tar directory entries bypassed every size cap.** A single directory entry declaring 8 GiB
      hit `continue` before `validate_expanded_size`, and tar advances past a skipped entry by
      reading and discarding its declared size: 1.25 MB in, 8 GiB decompressed, ~11 s of CPU, no cap
      fired. Size is now accounted before the branch, and every tar stream is additionally wrapped
      in a `BudgetedReader` — the only place that can bound the GNU longname/PAX `read_all`, which
      allocates inside the tar crate's iterator before any per-entry check exists.
- [x] ArcGIS feature pages were fetched under the 512 MB document ceiling and then JSON-parsed; they
      now use the 16 MB JSON ceiling like every other JSON call site (`client.rs`).
- [x] **DNS rebinding could redirect an approved URL onto the loopback or LAN interface.**
      `validate_remote_url` resolved a hostname, rejected any non-public answer, then discarded the
      addresses and handed the *hostname* to reqwest, which resolved it again at connect time. A
      resolver answering public-then-private walked straight through. The client now installs
      `PinnedDnsResolver` (`client.rs`) as its `dns_resolver`: `validate_remote_url` pins the answer
      it approved, and the connector can only use pinned addresses. The hostname still drives TLS SNI
      and the `Host` header, so virtual hosting is unaffected; unpinned names fail closed; pins carry
      a 120 s TTL and a 512-entry bound. `lookup_host` gained a 5 s timeout.
      Also closed in the same function: `is_public_ipv6` judged only `::ffff:` mapped addresses, so
      NAT64 (`64:ff9b::/96`, `64:ff9b:1::/48`), 6to4 (`2002::/16`) and the deprecated
      IPv4-compatible `::a.b.c.d` form all reached 127.0.0.1 and RFC 1918 space untouched.
      4 tests. `the_client_refuses_to_resolve_a_host_that_skipped_validation` is the load-bearing
      one: it drives a real `send()` and asserts on the resolver's own refusal message, proving the
      connector cannot reach the system resolver at all — which is what makes rebinding impossible.
      It does not simulate a rebinding attack; no test here does.
- [x] **Five analytics panels reported findings for a query that never ran.** A failed
      `refreshAnalytics` logged and returned, leaving every panel on its empty state: "No fast-drop
      signals detected", "No risk signals to summarize yet", "Relationship map ready", and a green
      "Live · 2m ago" freshness badge over a figure that had stopped updating. In an evidence tool
      those are findings, and the app had no evidence for any of them. Added `analyticsError` to the
      store, set on the failed refresh and cleared by `setAnalyticsData`, plus
      `components/charts/AnalyticsUnavailable.tsx` — an "unavailable" state distinct from empty —
      wired into `AnomalyRadar`, `LifecycleFunnel`, `PerformanceScatter`, `TemporalMap`,
      `MetricsGrid` and `OverviewDashboard`/`SignalSummary`. Panels holding real data keep rendering
      it and the freshness badge turns amber "Stale · last updated …" instead of hiding it.
      Verified by driving the real UI: `npm run dev` in a plain browser makes every `invoke` throw,
      which is exactly this failure path. All six surfaces render "unavailable"; none of the
      all-clear strings appears.
- [x] Telemetry union drift is now a build failure. `scripts/check-ipc-contract.mjs` compares
      `IngestionProgress` against `src/types/investigation.ts` and names the missing variant. This
      resolves `TODO(telemetry)` — and caught a variant I had added on only one side.

### Outstanding — audited, evidence-backed, not yet implemented

Three audits ran this session. Their surviving findings, highest severity first. Each was verified
at the line; two carry running proof-of-concept binaries.

**Ingestion and parsers**
- [ ] `queue.rs:1546` / `:2418` — **this finding was wrong as written; re-audited, see below.**
      The claim was that the host allowlist is "never applied" on the download and JSON-derived-link
      paths. It is applied: `guidance_allows_url` (`queue.rs:3903`) does enforce `allowed_hostnames`
      whenever an operator configures it. What is true is that the list is empty by default, so by
      default a crawled page can steer a download to any host.
      The proposed fix — copy the follow-path gate at `:1583-1587` — must not be applied. That gate
      is `source_host_matches || guidance_explicitly_allows_host || is_supported_external_data_platform`,
      and `is_supported_external_data_platform` (`queue.rs:3959`) lists ArcGIS, Tableau, Google,
      Socrata, GitHub, Box, Dropbox, SharePoint and OneDrive — not S3, Azure Blob, Granicus or
      CivicPlus, which is where government portals actually host their PDFs. Applying it would
      silently stop ingesting most real portals' documents.
      Whether cross-host downloads should be blocked by default is David's call: either default
      breaks something real. The alternatives are (a) leave permissive and record provenance,
      (b) restrict to the source's registrable domain, which needs a public-suffix list,
      (c) an explicit `follow_cross_host_downloads` policy flag, and a default has to be chosen.
- [ ] `queue.rs:1561` (`abort_workers`) — cancellation does not stop in-flight extraction.
      `spawn_blocking` tasks cannot be aborted, so the pause and remove paths write CANCELLED and
      return while extraction runs on.
      Thread an `AtomicBool` into the tar/zip/OCR loops.
- [x] `.msg` nesting now spends `container_depth`; parser export identity and format detection use
      the same filename, and the obsolete durable backstop field is compatibility-tested.
- [ ] `queue.rs` container expansion — per-member and per-depth bounds are enforced, but no single
      aggregate byte budget spans an entire recursively extracted tree. Committed attachment and
      container trees are retained evidence, not disposable caches; lifecycle policy must preserve
      references rather than deleting these paths blindly.
- [x] Structured JSON parsing and recursive discovery were moved off the async runtime and now
      return only bounded scheduling summaries.
- [ ] Lower: uncapped in-memory `visited_urls`; deterministic (unjittered) retry timing;
      non-persisted scheduler counters on restart; and untrusted SQLite opened without
      `cell_size_check`/defensive mode (`database.rs:38`). `Retry-After` is already capped at 120
      seconds, robots policy is fetched/cached/enforced, and abandoned uncommitted staging is now
      reconciled safely at startup.
- [ ] `source.rs:45` `BUG(evidence)` — assessed **not a real defect**: the code already re-stats the
      open descriptor and re-verifies the byte count. The residual race needs local write access to
      the evidence store, which is outside the untrusted-input threat model. Remove the marker.

**Frontend**
- [x] `LegalCompliancePanel.tsx` — reports unavailable rather than "No active findings recorded"
      when the findings query failed.
- [x] `App.tsx` — a failed `list_saved_portals` has an explicit unavailable state and never shows
      first-run onboarding.
- [x] `SearchFilter.tsx` / `MainLayout.tsx` — a failed search retains and labels the last successful
      results, and an empty failed query reports unavailable rather than blaming the filters.
- [x] `App.tsx` — identical failures from the 1500 ms runtime poll are logged immediately and then
      at most once per minute, with a recovery event, so they cannot rapidly evict the diagnostic
      activity that preceded the outage.
- [x] Async result publication is centralized through `useLatestRequestGate`, including the six
      originally identified surfaces and related search/export/source operations that publish into
      the same state. Superseded `finally` blocks are guarded as well as success/error branches.

**Storage**
- [ ] `duckdb.rs` `TODO(concurrency)` — write serialization and live sibling reads are correct, but
      an analytics query during heavy ingest waits on the mutex with no timeout, queue-depth signal,
      or cancellation.
- [ ] `SqliteRepository` → `DuckDbRepository` rename. The alias is gone from `setup.rs`; the
      remaining call sites are the command modules' `State<'_, SqliteRepository>`. Do this alone —
      it touches every command signature.

### Blocked on David — needs a commit decision, not a code change

- [x] **The earlier fresh-clone blocker no longer applies.** The previously untracked application
      files (`src-tauri/tauri.conf.json`, `src/ipc.ts`, `index.html`, `eslint.config.js`, both
      lockfiles, icons, templates, and Rust modules) are present in the current tracked tree, and
      the stale root `tauri.conf.json` is absent. The pre-existing `analytics-unavailable.png`
      screenshot was removed at the user's request; the continuation changes are tracked and
      committed in ordinary history rather than being required from an untracked worktree.
- [x] Resolved by renaming to `docs/ROADMAP.md`; `git check-ignore` now exits 1, so a clone sees it.

## Architecture decisions

### One corpus, one extraction pipeline (decided 2026-08-25)

Do **not** split the frontend by data type: no per-type workspace, each with its own extraction
pipeline. The value of this tool is cross-source correlation, and a per-type split destroys it. The
strain that motivates the idea is in acquisition (`src-tauri/src/ingestion/queue.rs`, 4,435 lines),
not in the frontend, so splitting the frontend moves nothing. The direction for acquisition is an
adapter per acquisition family, and the binding constraint there is that the durable frontier
persists `IngestionTask` as JSON: an adapter split must keep **one** task enum and move only
behavior into adapters, or in-flight rows need migrating.

### Window shape: detached document windows, not tabs (decided 2026-08-25)

The question was what "different tabs which open different frontend windows" should mean. Four
options were weighed against what the code actually is today.

Today the app is one window. `App.tsx` owns the `ingestion-telemetry` listener, a 1.5-second
`list_portal_runtimes` poll, and the analytics refresh. `useAppStore` is a single global zustand
store, unpersisted apart from the theme. `workspaceView` (`overview | patterns | evidence |
activity`) shows exactly one view at a time, and a document opens as a full-screen modal inside
`MainLayout.tsx:257` that Escape closes.

- **Rejected — a tab bar for workspace views.** The sidebar already switches `workspaceView`. A tab
  bar renames that control; it still shows one view at a time and adds no capability.
- **Rejected for now — multiple full `App` windows.** Each Tauri window is a separate webview with
  its own JS context, so each would run its own 1.5s runtime poll and its own telemetry accounting,
  and each would hold a *separate* zustand store. There is no cross-window state sync in this
  codebase and nothing cheap to add: adding a source in window B would not appear in window A until
  its next poll, and the sync logs and session metrics would diverge permanently. This is the option
  the earlier note called "cheap and orthogonal". That was wrong about this variant; it is true only
  of the leaf window below.
- **Deferred — a non-modal docked document pane.** Cheaper than the decision below and it would put
  the activity log beside a document, but it cannot put two documents side by side and it fights for
  room against a 900px `minWidth`. It does not conflict with the decision; build it later if wanted.
- **Decided — detached, read-only document windows.** One extra Tauri window per open document,
  rendering `DocumentViewer` (plus `UnredactorPanel` for PDFs) and nothing else.

The reason this one is genuinely cheap, verified by grep rather than assumed:

- `DocumentViewer`, `UnredactorPanel`, and `useC2PA` reference neither `useAppStore` nor `listen(`.
  The viewer subtree is store-free and listener-free; it takes `documentId` and `portalKey` and
  fetches everything else over IPC itself. A detached window therefore needs no store, no telemetry
  listener, and no poll.
- App-defined `#[tauri::command]`s are not ACL-gated: `get_document_text_chunk` appears 0 times in
  `src-tauri/gen/schemas/acl-manifests.json`, which lists only core and plugin permissions. The new
  window needs no capability entry to load a document. `capabilities/main.json` is scoped
  `"windows": ["main"]`, so it would only need extending if the document window ever calls a *core*
  API (window controls, events, `set_app_theme`).

It also delivers the two things one window genuinely cannot do: two documents side by side, and
reading a document while the activity log stays visible.

To reverse: delete the window-creation command, the document-window root branch, and the button.
It touches neither the store, the corpus, nor acquisition.

**Not yet built.** Proving a Tauri window actually spawns needs `npm run tauri dev`, i.e. a full
cargo build of a tree a concurrent session is mid-edit in. Notes for whoever builds it:

- Create the window from Rust (`WebviewWindowBuilder`, `WebviewUrl::App("index.html?document=…")`)
  rather than from JS. `WebviewWindow.new` in JS would need `core:webview:allow-create-webview-window`
  added to the main window's capability; a Rust command needs no ACL entry, and the command then
  goes through `src/ipc.ts` and `scripts/check-ipc-contract.mjs` like every other command.
- Window labels are unique. Reopening the same document must find-and-focus the existing window, not
  create a second one with a colliding label.
- A Rust-built window does **not** inherit `transparent`, `decorations`, or `titleBarStyle` from the
  `main` entry in `tauri.conf.json`. Set them on the builder.
- The document-window root must replicate `App.tsx`'s wrapper (`app-theme`, `data-theme`,
  `data-platform`) or the styling drops. Theme can be read from `localStorage` (`onrecord-theme`),
  which is shared across windows by origin; `set_app_theme` is a core API and is not needed.

## Query system

- [x] Define `QueryAst`, filters, issues, and source modes.
- [x] Parse readable qualifiers (`field:value`, quotes, negation).
- [x] Serialize a stable qualifier string.
- [x] Add deterministic natural-language scaffolding for dates/status/response.
- [x] Add a compatibility adapter that reports deferred filters.
- [x] Show recognized qualifier chips in evidence search.
- [ ] Add a typed `QueryAst` IPC payload to the Rust query command.
- [ ] Resolve agency names and portal names through loaded indexes.
- [ ] Enforce date, document, type, portal, negation, and boolean filters in Rust.
- [ ] Add parenthesized groups, `AND`/`OR`, ranges, and parser diagnostics.
- [ ] Add a visual filter-row editor with keyboard-accessible chips.
- [ ] Add natural-language interpretation preview and confidence states.
- [ ] Add saved searches, import/export, and shareable query links.
- [ ] Add parser property tests and fixture tests for malformed input.

## Frontend polish

- [x] Native-feeling light default with dark/automatic appearance modes.
- [x] Platform-aware macOS/Windows density and keyboard labels.
- [x] Command palette with workspace, source, sidebar, and appearance actions.
- [x] Consolidated instrument strips for overview, operations, and corpus metrics.
- [ ] Add a first-run walkthrough that can be replayed from Help/Commands.
- [ ] Add empty-state sample data previews without implying live records.
- [ ] Add reduced-motion and high-contrast preference controls.
- [ ] Run keyboard-only audit across every workspace and modal.

## Data and safety

- [ ] Keep unsupported query filters visible as warnings; never silently drop them.
- [ ] Add query execution telemetry without storing sensitive query text.
- [ ] Add cancellation/debounce behavior to the typed query IPC path.
- [ ] Add maximum-cost guardrails for broad corpus queries.
- [ ] Verify exported legal briefs include the exact serialized query and time.

## Release

- [ ] Run the full Rust test suite and IPC contract check after query IPC changes.
- [ ] Validate light/dark/automatic appearance on macOS and Windows builds.
- [ ] Test standalone window restoration, menu shortcuts, and deep links.
- [ ] Run accessibility contrast and focus checks at 100%, 125%, and 200% scale.
- [ ] Package, sign, and notarize platform artifacts.
