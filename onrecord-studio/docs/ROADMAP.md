# OnRecord Studio roadmap and handoff ledger

> Renamed from `docs/TODO.md`, which matched a `todo.md` pattern in a contributor's
> personal global gitignore and was therefore invisible to every clone.

This file is intentionally explicit so work can resume safely after a rate
limit, context compaction, or a change of contributor. Search for `TODO(` and
`NOTE(` in `src/` for the code-adjacent decisions.

## Session log

### Product focus — 2026-09-13 San Diego

The active product direction is [San Diego first](san-diego-focus.md): City records,
CPP meeting preparation, and SDPD policy research. The home screen and source
presets now reflect that scope. New general-purpose adapters and jurisdiction
expansion are deferred until the local acquisition → search → document → briefing
workflow meets its acceptance milestones. Older entries below remain a historical
ledger and do not override this priority.

This pass addresses the five workflow defects in the
[September 13 investigation](investigation-2026-09-13.md), adds direct routing for
known City document pages, and preserves SDPD category and pagination identities.
See the focus document and `san_diego_live` tests for source coverage and live checks.

### Continuation — 2026-08-29 unreached-surface pass

Verification at the end of this pass, all run together: `cargo fmt --check`,
`cargo clippy --lib --all-targets` (clean), `cargo test --lib` (302 passing), `npx tsc --noEmit`,
`npx eslint src --max-warnings 0`, `npx vitest run` (99 passing), `node scripts/check-ipc-contract.mjs`
(38 commands, 18 telemetry events, 6 verification states). The counts recorded in the older entries
below are stale by construction: they were 217/226/229 Rust and 61/70 frontend against 30 and 27
commands.

The theme of this pass is that a command with no caller is not dormant, it is untested. Three
defects were found by writing the first caller for one of them, and every one of them was in code
that compiled, linted, and passed 283 tests.

- [x] **A statutory extension could not be recorded at all.** `record_statutory_extension` set
      `recorded_at` from `current_timestamp` inside `ON CONFLICT ... DO UPDATE SET`, where DuckDB
      binds a bare identifier against the target table and refuses the whole statement, so it failed
      on the first insert. `statutory_extensions` has exactly one production writer and the legal
      path consumes it: `refresh_request_timeline_audit` feeds `evidence.extension` into
      `evaluate_statutory_compliance_with_determination_at`. A request whose agency lawfully invoked
      a CPRA extension was therefore evaluated against the initial period and could be reported as
      exceeding a statutory deadline it had not.
- [x] **`record_source_task_failure` had the same defect**, on the live ingestion path, which means
      no ingestion failure was ever recorded on any build that shipped it. The guided-repair surface
      reads that table.
- [x] **Every stored timestamp was unparseable and in the wrong zone.** `%f` is microseconds, so
      `'%Y-%m-%dT%H:%M:%fZ'` dropped seconds entirely and wrote microseconds into the seconds field;
      `TRY_CAST(... AS TIMESTAMPTZ)` returns NULL for the result and `parse_from_rfc3339` rejects it.
      Two of the affected columns, `evaluated_at` and `imported_at`, are copied verbatim into an
      exported legal brief. Separately, `strftime` renders a TIMESTAMPTZ in the session zone while
      the stored literal `Z` claims UTC, so every value was local wall-clock time labelled UTC. One
      format at all 27 call sites and nine schema DEFAULTs, `TimeZone` set to UTC on the connection,
      and a source-level test pinning both spellings.
- [x] `get_request_statutory_extension` plus the recording surface in `LegalCompliancePanel`. The
      write is an upsert, so a failed read renders as unavailable and withdraws the control rather
      than offering to replace evidence it never saw. Its staleness test was proven by removing the
      guard.
- [x] Motion and contrast preferences, resolved like the theme and reachable from the palette.
- [x] `start_portal_sync` and `mask_sensitive_text` unregistered.
- [x] **`import_auxiliary_events` has a caller.** It was the only production writer for
      `auxiliary_events`, which `anomaly.rs:43` and the cross-agency correlation queries read, so
      unregistering it would have left live analysis over a table nothing could fill. The surface is
      `AuxiliaryEventImport`, opened from the temporal correlation panel whose empty state was
      already inviting the data: a CSV or JSON export, header-detected column mapping the operator
      can correct, a preview naming every row that will not land and why, and the whole original row
      kept as `sourcePayload`. Rows the backend would drop as `rejectedCount` are refused and
      explained before the import commits, not counted afterwards. A successful import raises
      `analyticsRefreshRequests` so the aggregates re-run rather than continuing to report a
      coverage answer the database no longer supports.
- [x] **Rows written by earlier builds keep the old timestamp format and zone.** The lost seconds
      cannot be recovered by any backfill, which is why there is none. These columns are compared
      lexically rather than parsed and the date and hour dominate that comparison, so
      day-granularity filtering is unaffected; within-minute ordering is not.
      **Decided 2026-08-29: the export neither refuses nor prints a malformed instant as one.**
      Refusing would withhold a whole legitimate brief over one permanently unfixable legacy row;
      printing verbatim states a time the record never established. `evaluated_at` and `imported_at`
      in the signed package are now `RecordedInstant`, an `INSTANT`/`UNPARSEABLE` tagged form that
      carries the recorded text either way, on the same rule as `RecordedDigestSource::None` and the
      nullable `documents.shannon_entropy`. The package schema is `onrecord-evidence-package-v2`;
      nothing in the tree consumed v1, and the human-readable brief never printed either column.
      Reversing the decision is deleting one enum and two call sites.
- [x] **`start_page` and `per_page` are reachable again, and now survive a resume.**
      `start_data_source` fixed them at 1 and 50 once `start_portal_sync` was unregistered, so a
      source needing a different slice of a portal's request index had no way to say so. They are
      one `MetadataPageWindow` on the command, set from the crawl settings dialog, bounded in the
      dialog against `validate_page_parameters` and re-checked in Rust. Writing the setter exposed
      the reason it matters: `start_task_pool` enqueues a resume's seed unconditionally alongside
      the adopted frontier, and `resume_registered_source` hardcoded page 1 at size 50, so an
      operator starting at page 400 would have had the first 399 pages crawled on the next launch
      and on every startup recovery. The window is therefore persisted on
      `source_connector_recipes` and read back by both resume paths. The column is nullable with no
      DEFAULT because 1 and 50 are legal choices: NULL says the operator never chose, which is not
      the same statement as choosing the built-in window, and a recipe row that predates the column
      must keep saying the first thing. `used_saved_guidance` is now derived from the recipe's
      contents rather than from a row existing, or a window-only row would have claimed guidance
      was applied that never was. Four proofs by removal: the resume seed, the two writers not
      overwriting each other, the pre-column row reading as absent, and the untouched dialog
      sending `null`.
- [x] **The window was keyed on the operator's pasted URL and read back by the canonical one, so
      no resume could ever find it.** An adversarial review of the commit above found this in three
      independent dimensions. `canonical_url` is `Url::to_string()`, which supplies the path a
      pasted bare host omits, so `https://city.nextrequest.com` is stored and
      `https://city.nextrequest.com/` is queried, and both resume paths fell straight back to 1 and
      50: the exact revert the feature exists to prevent, now silent. Every one of the four tests
      missed it because each used the same literal on both sides. The writer now takes the
      `PortalRegistration` rather than a key and a URL, so the pasted string is not in reach.
      The same mismatch on guided-repair recipes is closed below.
- [x] **A window-only row silently withdrew an approved publisher recipe.**
      `load_source_connector_recipe` took the highest-ranked row whole, and a NextRequest source
      that set a window now has a row of its own carrying default policy and guidance. A source
      running under a recipe its operator approved for reuse would have dropped to the built-in
      crawl policy the moment a window was set, with nothing said. The loader now reads both
      candidates and takes the guidance from whichever row carries any, while taking the window
      only from an exact `root_url` match: a window says where in one portal's request index to
      start, which is not a fact about a sibling that shares a publisher.

- [x] **A guided-repair recipe had the same raw-versus-canonical key mismatch, and had it longer.**
      `discover_and_start_public_source` saved it under the pasted string while `resume_source_sync`
      and startup recovery read by `canonical_url`, so an operator's guidance applied when the
      source was re-added and stopped applying on every resume and every launch, with
      `used_saved_guidance` reporting it accurately in the first case and not the others. Moving the
      write key alone would only have traded one silent miss for another: `start_data_source` reads
      the recipe *before* discovery, because the guidance it holds is an input to discovery, so a
      pasted URL is genuinely all it has.

      So the two readers were separated rather than reconciled. The resume paths hold the
      registration and now match `source_key`, the primary key, which has always been written and
      read the same way; that finds rows already stored under a pasted URL, so nothing needed
      rewriting. The URL reader stays for `start_data_source` and normalizes its argument the way a
      submitted URL is normalized on the way in, so a pasted bare host finds the canonical row. The
      writer takes the `PortalRegistration`, which also repairs `publisher_pattern`: it was parsed
      out of the pasted string, so a paste with no scheme silently stored no pattern and withdrew
      the source from publisher reuse. The URL reader matches the argument as typed *and*
      normalized, because normalization is not idempotent against what is already stored: rows
      written before this hold the raw pasted string, and normalizing the query alone would have
      stopped finding them, closing two misses and opening a third on the one path that worked. Three proofs by removal: the legacy row read through the
      resume path, the two pasted forms read through the re-add path, the reclassified source, and
      the failed write that must not destroy the recipe it would have replaced. The re-add
      assertion on the legacy row is what caught the normalization regression, which the two tests
      written first could not see: one read a legacy row only by key, the other read pasted forms
      only against a canonically stored row.
- [x] **A source that reclassified at an unchanged URL failed its own start.** `root_url` is
      `UNIQUE` while both recipe writers upsert on `source_key`, and the key is a digest over the
      source kind and the final URL, so a page that begins advertising a different connector
      produces a new key at the same URL and an insert that collides. `portals.base_url` is not
      UNIQUE either, so a URL that later discovers as NextRequest registers a second key beside the
      first and reaches the same collision. Both writers now drop the row holding their URL under
      the other key, in the same transaction as the write that replaces it: a delete that ran on
      its own would leave a source with no recipe at all whenever the write behind it failed. A
      still-registered source at that URL does lose its stored guidance, which is the lesser cost;
      the alternative is that the start the operator asked for fails outright. It had been judged low and left, on the grounds
      that the key derives from the host; it derives from the source kind as well, and keying the
      row on `canonical_url` made the collision reachable enough to close here.

- [x] **A recipe is found from a URL that redirects.** `start_data_source` reads the recipe before
      discovery has run, so it holds the string the operator typed while the row is keyed on
      `canonical_url`, the URL after redirects. Normalization closed the bare-host and
      trailing-slash difference and could never close this one: no rule applied to a submission
      produces its destination. So the submission is stored, not derived. `submitted_url` is a
      nullable column with no DEFAULT written by both recipe writers, and the URL reader matches
      both columns against both query forms. It is not backfilled from `root_url`: that would
      assert the submission did not redirect, which is the one case the column exists to record.
      Rows written before it hold NULL and stay reachable through `root_url` exactly as they were.

      The lookup now ranks the match in SQL and selects the rank, rather than recognizing its own
      row by the column value that came back. A row matched on `submitted_url` carries a `root_url`
      equal to neither query form, so the old shape would have returned that row's recipe and
      silently dropped its page window. That is the two-arms-disagree failure the previous commit
      was written to repair, and it would have returned one layer down.

      The DELETE both writers run to clear a superseded `root_url` is deliberately not extended.
      `submitted_url` carries no UNIQUE, so no insert can collide on it, and dropping another
      source's row for sharing a pasted string would destroy that source's guidance for nothing.
      Not being UNIQUE is a fact the reader has to answer for, because a portal that moves resolves
      one pasted string to a new canonical URL and so to a second row legitimately holding it:
      `updated_at DESC` inside the own-row rank returns the source the operator most recently
      started there. Three proofs by removal: the submitted-URL arm, the rank that arm is given,
      and that ordering.

      Not closed by this, and not a defect: the NextRequest arm of `start_data_source` never reads
      a stored recipe at all, so a re-add of a NextRequest source runs the window the operator
      supplies now rather than the one they chose before. Applying a stored window silently to a
      start the operator did not ask for it on is a decision about what a re-add means, and it is
      not this change.

- [x] **A NextRequest resume re-fetches its start page every time.** A resume no longer seeds the
      request index when the frontier it adopted still owes a page. Pagination enqueues page N+1
      in the same durable batch as page N's documents, and page N's row is deleted only after that
      batch is written, so a surviving page row is what an interrupted walk looks like on disk. A
      source paused at page 900 carries page 901 and resumes there.

      Not the high-water mark this entry proposed. `fetch_requests` sends no sort parameter, so a
      page number is a position in whatever order the portal returns and not a cursor; persisting
      one and seeding past it would also mean a source that finished its walk never looked at its
      first page again, which is the only way records filed since the last run are found. The
      frontier already holds the fact, and holds it as work rather than as a guess.

      Two starts deliberately keep the seed. An operator-requested start is an instruction, and
      the window they just chose outranks an inherited frontier. A walk that ran to the end leaves
      no page row, so the resume re-enters at its start page exactly as before. `PoolSeedPolicy`
      is what separates the two, and `pool_seed_tasks` decides without provisioning a pool; the
      prefix the frontier is queried by is built beside `IngestionTask::target_id`, because a
      drift between them would fail no test of the decision and would silently re-seed forever.

- [x] **Entity extraction states what verified it, and two validating extractors were added.**
      Corpus schema 6 to 7, which discards and rebuilds the corpus index by design; it is a
      projection of the core database and reconciliation refills it. A test builds a file stamped
      version 6 with the old `corpus_entities` shape and proves it is discarded rather than fatal,
      because the fixture that already covered this built a file with no version marker at all and
      so never exercised the mismatch path.

      Every mention now records `extractor_id`, `extractor_version`, and an `EntityValidationGrade`.
      A confidence number could not carry this: 9,000 basis points on a host that `Url::parse`
      accepted and 9,000 on an uppercase run that `TODO` satisfies just as well are the same number
      and are not the same claim. Every entity records an `EntityResolutionBasis`, and every edge a
      `RelationshipBasis` — co-occurrence is `COMPUTED`, because the app derived it and can
      re-derive it, and calling it observed would assert something the documents never said. Only
      `EXACT_NORMALIZED_MATCH` is written today; the columns exist so a looser merge or a
      model-inferred mention cannot arrive later without saying what it is, and adding them then
      would be a second rebuild.

      `EMAIL_ADDRESS` is extracted from the same scan as `EMAIL_DOMAIN` and over the same span, so
      the two can never disagree about where the address is — the local part was already read to
      find the domain's start and was then discarded, and a domain cannot tell two correspondents
      at one agency apart. `MONEY` validates rather than matches: `$1,00,000`, `$12.3` and
      `$12.345` are rejected, because reading the last as `$12.34` would hand an investigator a
      figure the bytes never contained.

      `ORGANIZATION` no longer ends a run on a month or weekday. The rule terminates the run
      rather than excluding calendar words from `is_title_token`, because the blanket exclusion
      closes the same case and starts `May Department Stores` at `Department`.

      Not added: `PHONE`. `storage/pii.rs` already detects phone numbers and the app masks them in
      search results, titles, document text, and exports. An entity whose `display_value` is the
      literal number would surface in the pattern explorer exactly what every other reader masks.
      That is a decision about the masking seam, not an extractor to write.

- [x] **The entity graph exists.** Step 3 and step 4 of
      `investigative-workbench-architecture.md` section 7, at their smallest honest size and with
      no schema change: the edges were already stored, and `basis` now travels with them.
      `list_top_corpus_entities` ranks by distinct-document count rather than mention count, because
      an entity repeated forty times inside one letter is a feature of that letter while one
      appearing in forty documents is a thread through the corpus. `get_corpus_entity_neighborhood`
      returns one entity's neighbours ranked by shared documents, asking for one row beyond the
      limit so `truncated` states a fact: a sparse entity and a cut list are different claims and
      the list alone cannot tell them apart.

      `EntityGraph.tsx` draws it in hand-rolled SVG, per section 9's refusal of a chart dependency.
      A computed arc is dashed and an unverified node's outline is dashed, so co-occurrence cannot
      read as an observed relationship and a `PATTERN_ONLY` lead cannot read as a parsed value.
      Layout is radial and deterministic rather than force-simulated: every arc in an ego network
      shares one endpoint, so a force run resolves label collisions and produces distances that
      mean nothing, while radius here reads shared documents.

      An entity's `validation_grade` is the weakest among its mentions, not the best. An entity is
      only as verified as its least-verified mention, and reporting the strongest would let one
      lucky match speak for the rest. An unrecognized stored `basis` reads as `COMPUTED`, the
      weakest of the three: a row this build does not understand must never be promoted to
      `OBSERVED`, which is the only value that lets an edge speak as strongly as the document.

      The neighbourhood fetch is gated on the entity it was requested for. Proven by removing the
      gate: the superseded result renders its neighbours around the entity the investigator had
      already clicked away from, under that entity's name.

Open, and specific:

- [ ] **Every persisted edge is `COMPUTED`, by policy, not by a missing extractor.** The second
      `relationship_kind` this item used to call for now exists: `RelationshipKind` has eight
      variants (`CoOccurrence`, `SignedBy`, `Approved`, `Cites`, `Promises`, `Delivered`,
      `Supersedes`, `Amends`), and `analysis/typed_edges.rs` (`derive_typed_edges`,
      `role_and_signer_mentions`) actively extracts the typed ones from explicit textual cues
      during indexing (`storage/corpus.rs`). What remains true is the basis: `insert_relationships`
      hardcodes `RelationshipBasis::Computed` for every row regardless of kind, with the comment
      "co-occurrence and cue-matched typed edges are derivations this app performed and can
      re-derive; calling either observed would claim more than a pattern match." Email threading
      (`In-Reply-To` against an ingested `Message-ID`, already read by
      `detectors/email_thread_gap.rs` for a different purpose) and archive containment remain the
      candidates for an edge backed by something stronger than a cue match, i.e. the first
      extractor allowed to write `OBSERVED`. The `basis` column and the reader that distinguishes
      it are already in place end to end (schema, write, and read-back are all covered by
      `storage/corpus.rs` tests); only the OBSERVED-worthy source is missing.

- [ ] **A crawl resume re-walks from its seed URL.** The same seed additivity, on the non-
      NextRequest arm of `resume_initial_tasks`: a resumed crawl re-enqueues its root at depth 0,
      and `visited_urls` is a fresh set per pool, so pages already walked are re-fetched. Unlike
      the request index there is no single durable row that means "this walk was interrupted" --
      a crawl frontier holds many pages at once -- so closing it is a question about what a crawl
      resume owes, not the same fix applied to a second branch.

### Continuation — 2026-08-27 release-hardening pass

Before the unfinished feature surfaces were restored, the last complete verification run was
green: `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets -- -D warnings`,
and `cargo test --workspace` (229 unit tests passing, 0 failing, plus doctests). Frontend
verification was also green: `npm run lint`, `npm run test:unit` (70 tests), `npm run build`,
and `npm run check:ipc` (26 commands, 18 telemetry events, 6 verification states). These counts
are historical evidence, not a claim about the restored worktree; verification must be rerun.

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
      crawl host-policy path. Scheduler page/resource admissions are now persisted per run and
      inherited when an interrupted frontier is resumed.
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
      build/IPC contract, and 70 unit tests were green; Rust fmt, strict clippy, all 226 unit tests,
      and doctests were green before the restoration pass began.
- [x] The restored frontend investigation components were completed in `19d6f23`. A keyword and
      structural sweep of `src/components` on 2026-08-29 found no fabricated operational text: the
      only module-scope arrays are a palette, two field-descriptor lists, and an enum, and the
      three components most likely to fake data are store-backed with real empty states.
- [x] The `NOT_IMPLEMENTED` placeholders were removed in `2add00d` and `0333c92`. Zero occurrences
      remain in `src` or `src-tauri/src`.
- [x] The overlapping commands were reconciled on 2026-08-29, but not by the IPC contract check,
      which compares the two sides for drift and says nothing about reachability. Four commands
      were registered with no caller anywhere in `src/`. `start_portal_sync` and
      `mask_sensitive_text` were unregistered as subsumed and unused; `register_statutory_extension`
      got a caller; `import_auxiliary_events` got its import surface on 2026-08-29.
- [x] Qualifier search no longer sends visible qualifier syntax as document text. Status, flag,
      portal, and uniquely resolvable agency qualifiers compile into the existing native fields;
      negated, duplicate, ambiguous, and not-yet-executable filters are visibly marked "not
      applied" and never silently change the search term. Unknown fields warn without becoming
      text predicates.
- [x] Closed run-state races: event updates require the run they describe, stale starts/stops and
      unscoped active-run stops are rejected, pause/removal carry the captured run, and periodic
      runtime snapshots reconcile the entire run map atomically. Initial telemetry-listener
      registration retries with capped exponential delay. Version recovery now has its own
      one-at-a-time memory semaphore.
- [x] Hardened arbitrary SQLite evidence with `SQLITE_DBCONFIG_DEFENSIVE`, `query_only`,
      `trusted_schema=OFF`, `cell_size_check=ON`, zero attached databases/worker threads, and the
      existing value/row/output ceilings. A direct connection-state regression pins every control.
- [x] Durable retries now add deterministic per-frontier jitter above the exponential floor,
      preserving the retryability and attempt ceilings while preventing rows that fail together
      from replaying in lockstep. Crawl/resource identity sets now stop growing at their configured
      page/resource budgets instead of retaining an unbounded set of discovered strings.
- [x] Recursive container work persists its managed root directory in every durable child task.
      The worker remeasures the real tree after each archive or message expansion and rejects trees
      above 2 GiB or 16,000 entries, so nesting and process restart cannot reset the aggregate cap.
- [x] Source stop now joins in-flight parser work before finalizing or deleting frontier state.
      Workers skip refill/self-retirement once cancellation is observed, avoiding both detached
      blocking writers and provisioning-lock re-entry during teardown.
- [x] Crawl page/resource admissions are atomic DuckDB counters on the sync run. Frontier adoption
      carries the prior run's counters forward; migration tests cover the zero backfill for runs
      written before the counters existed and restart tests prove an exhausted budget stays closed.
- [x] Analytics workspace lock acquisition has a five-second bound and an explicit busy result.
      The obsolete `SqliteRepository` compatibility alias and all misleading command/test state
      types have been replaced with `DuckDbRepository`.

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
- [x] `SourceGuidance.restrict_downloads_to_source_host` implements the 2026-08-29 decision. The
      earlier finding, that the host allowlist is "never applied" on the download and
      JSON-derived-link paths, was wrong: `guidance_allows_url` does enforce `allowed_hostnames`
      whenever an operator configures one. What was true is that the list is empty by default, so a
      crawled page could steer a download to any host with no way for a source to say otherwise.
      The proposed fix, copying the follow-path gate, was rejected and stays rejected: that gate's
      `is_supported_external_data_platform` lists ArcGIS, Tableau, Google, Socrata, GitHub, Box,
      Dropbox, SharePoint and OneDrive, not S3, Azure Blob, Granicus or CivicPlus, which is where
      government portals actually host their PDFs.
      The flag is spelled restrictively so Rust's `false` *is* today's behaviour;
      `SourceGuidance` is whole-struct JSON with `#[serde(default)]`, so no migration was needed.
      The threading described here was not: `context.portal.canonical_url` is already in scope at
      every call site, and the check belongs in `prepare_discovered_download`, the one place a
      remote URL becomes a fetch task, rather than in the predicate shared with the follow path.
      Refusals are counted under their own `off-source-host-download` rule, the source root and the
      explicit `allowed_hostnames` both stay admitted, and three tests pin the default, the
      restriction, and that link-following is unchanged. The flag is set per source from the guided
      repair dialog's scoping rules; a policy no operator can reach is the same defect as a command
      with no caller.
- [x] In-flight extraction is no longer detached during cancellation. Source teardown signals the
      workers, joins their current task, then finalizes the run; cancelled workers bypass frontier
      refill and self-retirement so they cannot re-enter the provisioning lock held by teardown.
- [x] `.msg` nesting now spends `container_depth`; parser export identity and format detection use
      the same filename, and the obsolete durable backstop field is compatibility-tested.
- [x] Container expansion enforces one 2 GiB/16,000-entry budget across the on-disk recursive tree.
      The persisted root path lets resumed child tasks apply the same budget after restart.
- [x] Structured JSON parsing and recursive discovery were moved off the async runtime and now
      return only bounded scheduling summaries.
- [x] Scheduler page/resource counters persist on each sync run and transfer during frontier
      adoption. Identity sets are bounded by the configured budgets; retries are jittered; SQLite
      evidence uses defensive/cell-size controls; `Retry-After` is capped; robots policy is
      enforced; and abandoned uncommitted staging is reconciled at startup.
- [x] Removed the stale `BUG(evidence)` marker: the source parser already re-stats the open
      descriptor and re-verifies the byte count.

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
- [x] Analytics workspace lock acquisition returns an explicit busy error after five seconds
      instead of waiting indefinitely. A focused contention regression pins the bound.
- [x] Renamed every command, ingestion, storage export, and test reference to `DuckDbRepository` and
      removed the obsolete `SqliteRepository` alias.

### Blocked on David — needs a commit decision, not a code change

Nothing here is currently blocking. The cross-host download default was decided reversibly on
2026-08-29 and is implemented; see the ingestion list above.

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
**Ruled 2026-08-29: the native `QueryAst` waits for the search rebuild.** Building it now would
create a third query grammar that the rebuild then deletes. Two already exist: the qualifier AST
lives entirely in TypeScript (`src/query/types.ts:30`; zero occurrences of `QueryAst` in
`src-tauri`) and compiles down to the six flat scalars `query_audit_records_page` validates
(`commands/query.rs:12-24`), which is why portal, status, flag, and agency *are* enforced natively
while dates, types, negation, groups, and ranges are not. Separately, `query_corpus_patterns`
already accepts a typed structured payload with date ranges and kind sets, enforced in Rust
(`types/corpus.rs:90-98`, `storage/corpus.rs:577-629`). Section 5 of
`investigative-workbench-architecture.md` rules that search is rebuilt rather than extended, and
the AST belongs to that rebuild. Whoever builds it reconciles with `query_corpus_patterns` rather
than adding a third grammar beside it.

- [ ] Add a typed `QueryAst` IPC payload to the Rust query command. *Deferred to the search
      rebuild, per the ruling above.*
- [ ] Resolve agency names and portal names through loaded indexes.
- [ ] Enforce date, document, type, portal, negation, and boolean filters in Rust. *Portal,
      status, flag, and agency are already enforced as scalars; the rest are deferred with the
      AST.*
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
- [x] Motion and contrast resolve like the theme: `system` follows the media query, an explicit choice overrides it in either direction, both stamped before first paint and reachable from the palette.
- [ ] Run keyboard-only audit across every workspace and modal. This one needs the running app, not a test run.

## Data and safety

- [x] Unsupported query filters are visible as amber "not applied" chips with a count and are never silently dropped (`SearchFilter.tsx:146`).
- [ ] Add query execution telemetry without storing sensitive query text.
- [ ] Add cancellation/debounce behavior to the typed query IPC path.
- [ ] Add maximum-cost guardrails for broad corpus queries.
- [ ] Verify exported legal briefs include the exact serialized query and time.

## Release

- [ ] Run the full Rust test suite and IPC contract check after query IPC changes.
- [ ] Validate light/dark/automatic appearance on macOS and Windows builds.
- [ ] Test standalone window restoration, menu shortcuts, and deep links.
- [ ] Run accessibility contrast and focus checks at 100%, 125%, and 200% scale. Contrast and non-colour cues were met in `2c04d1a`; the scale sweep needs the app driven and cannot be closed from a test.
- [x] The Mac App Store chain is built and asserted: sandboxed Apple silicon bundle, provisioning
      profile identity and expiry validated before packaging, every Mach-O checked, and the
      installer chain verified (`d0f5cab`, `60348c9`, `56bb2c6`, `94276ac`, `09ab5a1`, `dbcb619`).
      App Store submissions are notarized by Apple, so there is no separate notarization step.
- [ ] Windows has no release path at all. `tauri.conf.json` sets `targets: "all"` while
      `docs/release.md` contains no Windows signing, packaging, or appearance content. Decide
      whether Windows ships before treating the two boxes below as in scope.

## Version 2: learned models (deferred, 2026-09-30)

Decision: version 1 ships with what exists today (on-device `bge-small-en-v1.5` embeddings,
deterministic detectors and statistics, opt-in cloud LLM for research). No new model work
before release. Governing rule for everything below: a learned model's output is an observation
about the model, not the record, so models live in the recall layer (ranking, candidate
generation, triage) and never become a `Finding` without a deterministic check and an
observable confidence basis. Statutory, deadline, and latency evaluators stay deterministic.

Ordered plan, each step gated on the eval set before it:

1. Evaluation sets first: ~300 labeled documents for `DocumentClass`; 50–100 request → responsive
   document pairs (recall@10 for `hybrid_search`, semantic/lexical/fused reported separately);
   ~200 denial and extension letters grouped into template families; person/org spans on ~100
   documents. Wire each into an `--ignored` cargo benchmark that prints metrics, in the
   held-out-eval shape the retired `parsers/neural_inference.rs` had. Baselines recorded before
   any change.
2. Frozen embeddings plus tiny heads: a linear or kNN head over the existing embeddings,
   serialized with a content hash and versioned alongside `EMBEDDING_MODEL_ID`/`REVISION` so a
   model revision invalidates heads. Targets: document classification beside
   `analysis/document_class.rs`, paraphrase and template-family detection replacing
   `phrase_match` in the evasion and cross-corpus detectors, intent routing. Confidence basis is
   "matches N labeled exemplars at cosine ≥ threshold", never a probability. Retune
   `fuse_document_results` toward semantic-primary only if retrieval recall improves. Pin the ONNX
   CPU execution provider for any path feeding a finding; add a same-corpus-twice reproducibility
   test.
3. Claim-versus-data validation (qualitative statements checked against quantitative tables).
   **Shipped in v1 (2026-09-30)** as `analysis/claim_validation.rs` plus the
   `claim_data_mismatch` detector and the `CLAIM_CONTRADICTS_DATA` finding kind; see
   `docs/analysis_methods.md` §5 for what it does and does not claim. The plan below is kept as
   the design record. Original design: extend `analysis/assertion_index.rs` with quantitative predicates (budget balance:
   surplus/deficit/balanced; total; percent change; count) so a prose claim such as "the budget
   was balanced with no deficit" becomes an `Assertion` with a canonical value and a scope
   (fiscal year, fund, agency). Evaluate each against imported analytical tables
   (`OPERATING_BUDGET`, `BUDGET_DIMENSION`, regional data) through `execute_restricted_sql`
   using the existing financial mapping roles and `FinancialScope`; emit a finding only when
   the claim and the rows materially differ under `AssertionValue::materially_differs`, with the
   query id and rows as the basis. A claim whose scope cannot be bound to a table (ambiguous
   fund, missing year, no mapped dataset) is reported as unreconciled, never as a contradiction.
   No model is required; an LLM or trained extractor is optional later only to widen claim
   extraction, and never decides the verdict. Eval set: hand-verified claim/table pairs from
   already-imported San Diego budget and RIPA data.
4. Cross-document redaction audit (pre-publication leak assessment). Deterministic first:
   link documents by case number, date, thread headers, near-duplicate text; align a redacted
   span against its unredacted counterpart; constrain by box width → character count. Primary
   output is "recoverable, by method M, from documents X and Y, N-way agreement". The recovered
   value is a separate, logged, explicit reveal, withheld by default when the span's inferred
   category is third-party PII. Constrained fill-mask ranking over graph-derived candidates is
   the only learned component, added after the deterministic path ships. Eval set: real
   double-release pairs from the corpus. The retired `neural_inference.rs` rule stands: rank
   investigator-supplied or graph-derived candidates, never invent them.
5. One additional pretrained on-device ONNX model for person/org NER through the existing
   download, manifest-hash, and status flow in `storage/vector.rs` (same runtime as fastembed).
   Model-extracted entities carry a distinct `EntityValidationGrade`; validating extractors stay
   authoritative.
6. Conditional only: NLI cross-encoder to widen candidate assertion pairs for
   `contradictory_statement` (deterministic `find_conflict_pair` still decides); fine-tuning
   `bge-small` on mined request↔document pairs only if step 1's retrieval set shows retrieval is
   the bottleneck after step 2, shipped as a new `EMBEDDING_MODEL_REVISION` with a full re-index.

Verified 2026-09-30: the research LLM path sends masked text only. `research/retrieval.rs` runs
`mask_pii(..).masked_text` on every passage (lines ~127 and ~166) before `research/model.rs`
builds the outbound `messages` payload. Any new cloud LLM surface must go through the same
masking step.

## Version 2: map workspace (deferred, 2026-10-01)

Decision (owner, 2026-10-01): a map view where a user draws or picks an area and asks counted
questions of it — encampment abatements in a boundary, a heatmap of abatements, public restroom
locations, Get It Done requests per area, shelter beds, budget by council district. Deferred to
version 2.

Already present: GeoJSON imports keep each feature's `_geometry` and `_crs`
(`storage/tabular.rs`); facility coverage takes user-supplied latitude/longitude rectangles and
never infers location from an address (`types/facilities.rs`); monitoring rows carry coordinates.

Missing: point-in-polygon (DuckDB `spatial` is not loaded); an offline basemap (the CSP allows
`connect-src 'self'` only, so tiles must be bundled, or start with boundaries drawn without
tiles); a map renderer; a boundary-layer registry (council districts, community planning areas)
with source citations. Questions are a structured picker (layer x measure x area) over the
analytical datasets, not free text, until the v2 model work lands.

Rules: a record without coordinates is counted and shown as "not placed", never dropped; no
address geocoding unless an offline geocoder is added with its match quality shown; a drawn area
is an inspectable object that can filter Evidence and Leads and be added to a case.

## Open follow-ups (2026-09-30, after the audit sweep)

Small, cross-file items that audits confirmed but left for a later pass; each has a design in
the audit report it came from.

- `useFindings` (src/hooks) now carries the shared loader; FindingsInbox, PostIngestBrief,
  StoryDesk, BriefingWorkspace, SubjectDossier and SelfReview still load on their own. Move them
  over one at a time (each has bespoke filtering or dismissal state).
- Periodic portal re-crawl: build it on the peer session's watchlist storage once that lands; do
  not add a second diff store.
- Signed, importable case bundle: casework portable packets already sign and import; what is
  missing is carrying finding dismissals and the briefing project inside the packet
  (`CaseworkPacket` is `deny_unknown_fields` at schema version 1, so this is a version bump with
  an import test on a version-1 packet).
- Query packs (`export_claim_query_pack`) carry SQL and rows; the peer's evidence packets carry
  basis, hashes and quotes. When both have landed, one download should offer both.
- No deterministic regression tests for the queue counter/refill fixes (needs repository fault
  injection).

Triaged 2026-10-02 from an external model review (DeepSeek, docs-only, so each claim was checked
against code first). Built: phrase search, storage-version compression, source re-check with a
watch dimension, Apple Vision and PDFKit OCR on macOS (no external tools, so it can run in the
sandbox; unverified in a signed App Store build). PST mailboxes as bounded containers
(`parsers/pst.rs`). Built 2026-10-03: the PST reader runs in a child process, the app's own
executable re-invoked with a hidden first argument that `main` dispatches before Tauri starts
(`parsers/pst_isolation.rs`), because `panic = "abort"` makes `catch_unwind` useless. The parent
owns the staging directory, kills the child at a wall-clock timeout (120 s plus 1 s per 2 MiB of
source), re-measures what it wrote, and records a crash, abort or timeout as a failure of that
file, never as an empty mailbox. PSTs have their own caps: 8 GiB source (local import and parser
capability; network downloads stay at 512 MB), 50,000 files and folders written, 8 GiB written,
100,000 message rows; the expansion is named `_mailbox`, which gives its tree the generic
recursive budget on top for attached archives. Zip and tar keep their caps. Sandboxed self-spawn was verified only with
a minimal ad-hoc-signed C probe bundle, not the OnRecord build; a provisioned App Store build and
App Review's view of it are unverified. Still open on PST: `outlook-pst` 1.2.0 cannot open
attached messages and failed every message of the public `enron.pst` (recorded per entry, not
fatal); memory is bounded only by the timeout, since macOS does not enforce `RLIMIT_AS`; a PST
cached under the previous `_contents` name is re-expanded and the old directory is left behind.
Open on the re-check: an opt-in schedule (never on launch), and retaining the
re-fetched bytes behind a referencing row that purge releases. Deferred, with the evidence:

- Ingest-time C2PA reading: none of the 91 stored files in the real installation carries a C2PA
  manifest (every `c2pa`/`jumb` byte match was CSS or prose), and nothing outside purge reads
  `provenance_manifests`. Recording manifest state at ingest would write "no manifest" for the whole
  corpus with no reader. Revisit when a source publishes signed assets; record it as its own
  trigger, never as a `provenance_verification_log` row (an ingest-time digest equals the recorded
  one by construction, so it checks nothing).
- Multi-statute, event-based deadline engine (replacing `EARLIEST_RECORDED_ACTIVITY_PROXY` with a
  recorded determination event): legal scope, needs per-jurisdiction rules and counsel review.
- Citation span primitive with Intact/Drifted/Stale resolution: foundation for redaction production
  and transcript anchoring; large schema and viewer change.
- Redaction production with self-attack, and excerpted verifiable bundles: depends on spans.
- Outbound filing with proof of transmission: a new outbound trust boundary (mail/portal accounts).
- [x] Rewriting existing installations to storage `v1.5.0` (2026-10-03, `storage::storage_rewrite`):
  foreign-key-ordered catalog copy, per-table count and digest verification, atomic swap with a
  retained backup. Real core 5.3 GB to 1.6 GB, corpus 1.12 GB to 0.81 GB. Not surfaced in the UI:
  outcomes go to stderr only, since a telemetry event needs a new `IngestionProgress` variant.

Closed on 2026-09-30 (second batch): `count_findings` and `FindingsFilter.id`; Story desk and
dossier draft records requests (10da52e); cross-agency comparison view (604a610); response-time
claim detector (§12 of analysis_methods.md); redaction-consistency detector and the
`REDACTION_INCONSISTENT_ACROSS_RELEASES` kind (§13); methods page `docs/METHODS.md` built by
`scripts/build-methods.mjs` from the eval scorecards; per-document query pack export; the claim
sweep run end to end over stored documents in the eval set; `test_support::register_analytical_table`
shared by the unit and integration suites (feature `test-support`, enabled by the crate's own
dev-dependency).

Closed on 2026-09-30 (commits 342735f..43da2f5): ChangeFeed watermark, lens loading status,
`.msg` withheld-attachment record, dismissal race, crawled-page content type, CKAN/Socrata
pagination on a withheld total, PostIngestBrief stale rows and `discoveredAfter` filter, palette
live region, Story desk dossier button, and the claim-validation eval set
(`tests/fixtures/claim-eval`, 17 hand-verified cases over real FY22–23 City exports).

Product direction under discussion: rank leads by observable public-interest factors (a
"Story desk"), ingest public statements as a source class and run claim-versus-record checks
against them, and assemble a lead into a dossier with its evidence chain, falsification test
and gaps. Not started.

## Findings on real collections (measured 2026-10-03)

Measured with `ingestion::live_evaluation` (ignored test; `ONRECORD_EVAL_DIR`, `ONRECORD_EVAL_SUBJECT`,
`ONRECORD_EVAL_REUSE=1` re-runs detectors only) against sandiego.nextrequest.com, subject
"encampment": 140/142 requests with open/close dates, ~600 documents. Every finding the detectors
produced was false and has been fixed away (form blanks read as redactions, section citations as
FOIA stamps, police service-request stats as records response times, keyword-collided award IDs).
The detectors now produce 0 findings on this corpus. Next, in order:

1. `no_records_contradicted` by meaning, not word overlap. **Built 2026-10-03; produces 0
   findings on this corpus, by measurement.** 13/140 requests closed "No responsive documents"
   (closure notices are stored as documents). Word overlap matched them to unrelated releases
   under every token rule tried (all-8, 75%, extra stopwords), and is gone. The embedding model
   now only proposes candidates (closest-chunk cosine of each release against the denied
   request's description); a finding needs the release public, from another request with the
   same `agency_id`, and dated strictly before the denial's `closed_at`. Without an installed
   model or an embedded corpus the detector reports not-run and keeps its earlier findings
   (`RefreshSummary::not_run`). Method: `docs/analysis_methods.md` §16.
   Measured with the detector's ignored `measure_on_a_live_collection` (env `ONRECORD_EVAL_DIR`,
   `ONRECORD_EMBEDDING_CACHE`) on a copy of the eval-encampment-2 database: 660 documents, 99,083
   chunks embedded; all 13 denials loaded; the top-8 cosines per denial (104 values) were identical
   on two runs. Hand-judged, none of the 104 is a contradiction except possibly one. Generic
   homelessness email dumps (20-2226, 22-854) dominate every list at 0.62–0.85, and the highest
   cosine overall, 0.8531, is a COVID-email set against 22-3123 (a COVID-swab HR request). Two
   candidates passed every deterministic check, and both were judged false: 24-952 SDPD citation
   data against 26-6188 (0.7732, outside its date range), and an SDPD encampment-fire log against
   24-4110, which concerns UCSD protest arrests (0.7499). `SIMILARITY_THRESHOLD` is therefore pinned at 0.86, above every judged
   non-contradiction. It is a precision floor, and recall is unmeasured.
   The one plausible lead is missed: ESD released a Caltrans Delegated Maintenance Agreement to
   25-10478 on 2026-01-14, the day before Purchasing & Contracting closed 25-10350…10354 with
   "no responsive documents". It scored 0.8018 against 25-10350 (5th, below four email dumps) and
   is a different `agency_id`. Request-to-request similarity was also measured and did not separate
   it either: 25-10478 scored only 0.6592 against 25-10397, while PRA-boilerplate requests score
   0.96 against each other. Open: (a) whether a closure that says "The City of San Diego has no
   responsive documents" should be tested city-wide rather than per department; (b) a recall
   signal that a generic document cannot win on length, for example cosine relative to the
   document's median cosine across all denials, measured against this lead.
2. Rolling productions: 24/140 requests released in installments, 6 over 180 days (20-2226: 30
   releases over 809 days). Needs release events stored per request (timeline "Document(s)
   Released" bylines); a pattern finding, not a legal claim; never derive `determination_at`.
   **Done 2026-10-03:** timeline release events stored in `request_release_events`;
   `ROLLING_PRODUCTION` fires at ≥3 release dates spanning >90 days (analysis_methods.md §15).
   Re-measured from the live timelines: 24/142 have two or more release events (the 30 for 20-2226
   include 9 "Released to Requester"), 8 fire. Not yet confirmed by an `ONRECORD_EVAL_REUSE` run
   over a re-collected database.
3. Done: disposition counts per department (`analysis/dispositions.rs`,
   `get_department_dispositions`, table under Compare agencies). Classified from the stored
   closure notice by ordered word-phrase rules; "not recorded" (no notice) and "not classified"
   (notice no rule matches) are separate counts. On the same collection (142 requests, re-read
   2026-10-03): 88 released, 35 released in part, 3 withheld, 13 no records, 3 withdrawn, 0
   unclassified, 0 unrecorded. The earlier 87/33 figures came from a different run. Rules are
   measured only against San Diego's wording; other NextRequest portals' notices will show as
   not classified until a rule names them.

## Owner focus: homelessness accountability (2026-10-04)

Decision (owner, 2026-10-04): leads should concern homeless sweeps, shelter beds, encampment
enforcement, public restrooms, housing, property stored and returned versus discarded, human
rights, poor or ineffective spending, and unfair treatment, not generic service backlogs.
"NextRequest files are probably most relevant."

Built 2026-10-04: leads from SDPD's published stop records (methods §18), fetched by the app
(`RIPA_STOPS` and three companion tables). Measured with the app's SQL on the files published
2026-10-04 (stops 2024-01-01 to 2026-09-28):
- 38,041 of 263,488 persons stopped were perceived as unhoused. Custodial arrest 28.8% against
  13.4% of others; on an outstanding warrant 14.2% against 3.6%; without a warrant 17.0% against
  10.5%; citation 14.3% against 41.8%; property seized 7.4% against 2.2%.
- Most recorded reasons: local ordinance infraction 5,841, local ordinance misdemeanour 2,835,
  647(e) lodging 2,362, encroachment (1460(b) SH) 859.
- For the same recorded reason, arrest without a warrant was higher for people perceived as
  unhoused in 2 of 38 reasons compared (petty theft, battery). Counting warrant arrests it was 9
  of 38. The gap is mostly warrant arrests, which the lead states.
- Property seized from people perceived as unhoused: evidence 1,810, contraband 1,501, vehicle
  impound 212, safekeeping 151, abandoned property 32. Nothing published says where it went.

Built 2026-10-04: hours between a posted abatement notice and the abatement, from released logs
(methods §19). Measured with the app's SQL on the two logs in the 2026-10-03 collection (25-5806
notices posted 2023-08-01 to 2025-07-28; 26-772 abatements completed to 2026-01-28): 11,090
notices, 14,000 abatements; 2,067 places with exactly one of each; 22 completed within 3 hours of
posting, 101 within 24 hours, 1,811 from 24 to 72 hours, 144 later; median 29.3 hours; 11 recorded
completed before posting; 214 of 11,104 abatements in the notice period at a place with no notice
in the log. The collection is copied to `/Volumes/A/onrecord-eval/eval-encampment-2` (core.db
paths still point at the original temp directory; files are under `downloads/objects/`). It also
holds SDPD arrest and citation exports (26-6187, 25-7080, 22-2226 with race) and Urban Corps
cleanup invoices with labour cost and tonnage per day (25-8547); none is read by a check yet.

Built 2026-10-04: the feed proposes the records collection. When an imported open-reports file
lists Encampment reports and no abatement log is imported, "What stands out" offers to collect
sandiego.nextrequest.com for "encampment, abatement, homeless shelter, public restroom, stored
property" (`dataGaps.ts`, `collect`; started through `start_data_source`). It starts only when
asked, never under automatic upkeep. The two released .xlsx logs import through the same call
Data upkeep makes and reproduce the figures above
(`tests/data_discovery.rs::released_abatement_spreadsheets_import_and_reproduce_the_measured_hours`,
ignored; needs `ONRECORD_ENCAMPMENT_COLLECTION=/Volumes/A/onrecord-eval/eval-encampment-2`). Not
verified: the collection itself in the running app.

Built 2026-10-04: leads from released arrest and citation logs (methods §20). Measured with the
app's SQL on 26-6187 (2025-01-01 to 2026-06-22): 82,054 arrests and citations; 2,349 cite an
encampment, encroachment, lodging or public urination section (encroachment 1,704, encampment 268,
urination 206, lodging 194); 1,894 citations and 455 arrests, 259 of the arrests on those sections
alone. Recorded race among the 2,349 against all in the file: White 41.3% and 34.0%, Black 26.7%
and 14.9%, Hispanic 24.5% and 30.6%. 22-2226 holds only 56.55 citations (1,564, 2017 to 2021), so
it has counts and no comparison. 25-7080 overlaps 26-6187 in dates and is counted alone.

Built 2026-10-04: shared denial wording (methods §23). The three detectors now identify an
agency by portal and id (same-numbered agencies on two portals were merged or pooled; reproduced
by a test in each detector before the fix), name the agencies from `agencies.name`, and cite each
passage by its character range in the stored text. Every phrase occurrence is a passage (up to 8
per document, cut to its paragraph), so two reused paragraphs in one letter are two clusters.
`cross_corpus_shared_language` also reports reworded reuse across three or more agencies (60% of
content words shared, under 85% of five-word sequences), with no confidence because the 60% floor
is unmeasured. Statute-like phrases count only near wording that addresses a requester. Pairs
come from MinHash bands: on 16,000 synthetic passages the index and banded pass took 0.4 s
(shingles) and 1.3 s (word sets) where the all-pairs pass took 259 s and 94 s, with the same
groups. The cap stays at the newest 2,000 documents (the paraphrase detector's rose from 800).
Not verified: the detectors on a real collection, and the phrase lists' precision and recall.

Measured, not built:
- Open Encampment reports in Get It Done (2026-09-14 copy): 2,036; median age 7 to 14 days in
  eight districts and 903.5 days in District 4 (158 reports); 850 days for the 29 held by DSD.
- The owner's library holds 3 requests from sandiego.nextrequest.com. The 142-request encampment
  collection measured on 2026-10-03 was made by `ingestion::live_evaluation` into a separate
  directory and is not in the library.
- Contractor billing for abatement crews (Urban Corps invoices released to 25-8547), read by a
  one-off script, not the app: 2,013 crew-days from 2025-01-02 to 2025-09-30 in the sheets that
  share one layout, labour 2,076,060 and vehicle fees 295,206 dollars, 672.3 tons recorded, so
  about 3,527 dollars per recorded ton. 1,471 crew-days record no tonnage (1,066 blank, 405
  zero), costing 1,790,050. Unverified: the monthly files repeat earlier months' sheets (deduplicated
  by date, crew and labour cost), earlier invoices use other layouts and were skipped, a blank
  tonnage is not zero, and some crews (police support) may not haul. Needs a parser for the
  invoice layout before any lead.
- Closure times by year (2016 to 2018 closed files) cannot be read as a slowdown: files are keyed
  by close year and the programme began 2016-05-20, so early years cannot hold old reports.
- Where referred reports went is recorded in Get It Done's `referred` column (closed 2018:
  Environmental Services 12,598, Police abandoned vehicles 4,038, Caltrans 1,341); no check reads it.

Next, in order:
1. The abatement check only fires once the released spreadsheets are in the library as tables.
   The app can already do both: the Collection plan dialog collects a source for a subject
   (`start_collection`), and Data upkeep with automatic import turns released sheets into tables
   (`maintain_one_table`). Not verified end to end: collecting "encampment" from
   sandiego.nextrequest.com in the owner's library, and the two .xlsx logs through the import
   (the native test uses CSV with the same headers and time format). Nothing proposes that
   collection to the owner; the gap seeker only fetches City datasets. Then checks for the other released
   tables: SDPD citation and arrest exports (the columns `encampment_ordinance.rs` already
   reads), and contractor invoices (cost and tonnage per abatement day against the budget).
1a. A stated notice minimum: with the City's abatement procedure in the library as a stored
   authority, the abatement log counts could be compared with it as `encampment_notice.rs` does
   for notice documents.
2. Stored versus discarded property: no published table records it. The stop records give the
   seizure basis only. The check has to come from released abatement and storage logs (item 1).
3. Done 2026-10-04 for areas: Encampment, homeless, shelter and restroom services are shown in
   the area leads ahead of larger services (methods §17 B). By staff group (29 Encampment reports
   held by DSD, median 850 days) is not built.
4. Homelessness spending against outcomes: Homelessness Strategies and Solutions department lines
   in the operating budget against shelter capacity and exits, which need SDHC and RTFH tables
   that the source catalogs list only as pages and PDFs.
5. Local ordinance codes 65000 and 65002 do not name the ordinance; the stop records'
   `reasonforstopexplanation` text may. Not read yet.

## Cross-source accountability build (2026-10-04)

Decision (owner, 2026-10-04): implement the cross-source comparisons, evasion profiling, escalation,
briefing, displacement, provenance and change-detection ideas the owner supplied, finding data
sources where the library lacks them. Settled constraints, so they are not reopened: every output
is a count or a quoted statement with its source; no output says an agency intended, hid, or
broke anything unless a counted statutory deadline supports it; a comparison between sources is
not a causal claim. Not built, by decision: an anonymous source drop with automatic release (a
desktop app cannot provide source anonymity; a provenance manifest on a file stripped of
identifying marks attests receipt, not authenticity), a predicted settlement probability or
liability index (no data supports the figure), a verdict that new ordinance text conflicts with
state law (the app reports what the text changed), and anchoring hashes to an outside ledger by
default (opt-in only, and never submitted without the owner's confirmation).

Audited 2026-10-04 (file references in the session's audit reports; the facts that shape the work):
- A lead cites one snapshot and stores no citations, SHA-256 or SQL (`service_leads.rs`), so a
  cross-table lead cannot cite its numbers yet. (Fixed 2026-10-04, below.)
- The operating budget file has no council-district or programme column, and no published City
  table gives budget or spending by district. District-level funding statements can only be
  checked against a document that states them.
- Shelter capacity: UCSD Homelessness Hub ArcGIS layer `HIC_ES_2014_2024` (one row per
  emergency-shelter project per year 2014 to 2026, city, address, coordinates, beds, utilisation;
  sourced from RTFH's Housing Inventory Count; no licence stated). RTFH's own yearly workbooks
  cover every project type but change layout and URL each year. HUD's per-CoC PDFs are the
  reference totals (CA-601 emergency shelter year-round beds: 4,386 in 2023, 3,746 in 2024, 3,587
  in 2025). SDHC occupancy exists only as Tableau and PDF.
- Release timing: `documents.created_at` holds the portal's timestamp with offset; no detector
  reads weekday, hour or holidays; PDF and OOXML timestamps are compared only with the request's
  determination time.
- Redaction corroboration never searches imported tables and rechecks only the newest 200
  documents.
- Budget: no under-spending, adopted-without-actuals or transfer finding; operating actuals are
  not fetched by the app.
- Change detection: a source re-check records both hashes and discards the new bytes; nothing
  detects a request or document dropping out of a portal listing; no row or column comparison
  between dataset snapshots.
- Council dockets: fetchable from the City's OnBase agenda pages (meeting list, agenda body, item
  detail, attachments); the system publishes no posting time, so a revision is only visible by
  comparing the app's own successive copies. Settlements over 50,000 dollars are Council
  resolutions (paged index, one scanned PDF each); no payout table is published.
- Hand-imported files are stored with `visibility = public` and one detector counts them as an
  agency release.
- Escalation: no draft for a deadline that passed with no response, no draft cites other
  findings or the agency's record, no petition draft.
- Signed exports exist (Ed25519, self-managed key); nothing is signed per document, and the
  architecture doc's "no per-document retrieval time is stored" conflicts with
  `document_acquisitions.retrieved_at`.

Built 2026-10-04: leads cite their tables. Each stored lead lists every table it read (its own
first) with display name, source address, retrieval date and full SHA-256, then the SQL that
produced it (`service_leads.rs`, methods §17 E). The Findings inbox shows the text as written and
the dossier's Markdown export carries it. No citation rows or columns were added.

Built 2026-10-04: budget, Encampment reports and shelter beds by fiscal year (methods §21;
`SHELTER_INVENTORY` fetch, `capacityLeads.ts`, `CAPACITY_COMPARISON_LEAD`). Measured with the
app's SQL in the DuckDB CLI on files retrieved 2026-10-04 (budget_operating_datasd.csv; Get It
Done open and closed 2020 to 2026; HIC_ES_2014_2024, 719 rows, 384 for the City of San Diego):

| fiscal year | adopted expense, Homelessness Strategies & Solutions | Encampment reports filed | year-round emergency shelter beds (all beds), January count |
|---|---|---|---|
| 2021 | 64,235,899 | 34,288 | 3,225 (3,239) |
| 2022 | 18,071,824 | 47,364 | 2,434 (2,529) |
| 2023 | 25,157,607 | 60,005 | 2,517 (2,632) |
| 2024 | 44,007,454 | 69,987 | 2,476 (2,618) |
| 2025 | 53,916,571 | 48,135 | 1,970 (2,124) |

First to latest: budget down 16.1%, reports up 40.4%, year-round beds down 38.9%. In the 2025
count 2 of 37 projects record no year-round count and 1 no total. Not compared: fiscal 2018 to
2020 (no department with "homeless" in its name in the budget file) and fiscal 2026 (its budget
rows carry no budget cycle; 43,758 reports, 2,055 year-round beds). Also measured: 335,087
Encampment reports across the open and closed 2018 to 2026 files with no id repeated and none
without a filing date; none sits in a closed file earlier than the year it was filed; closed 2016
and 2017 hold no Encampment reports. The endpoints matter: the adopted amount fell from fiscal
2021 to 2022 and rose each year after, so the lead lists every year. Not verified: the fetch and
the lead in the running app (the native test imports fixture files with the publishers' headers,
not the downloads); whether the fiscal 2021 amount includes one-time funds; HUD's CA-601 totals
cover the whole county and were not compared.

Order of work:
1. Done 2026-10-04 (two paragraphs above): leads that cite several tables with snapshot, SHA-256
   and SQL; shelter inventory fetch; a lead setting homelessness budget, encampment reports and
   shelter beds side by side by year.
2. Shared denial language: agency identity fix, agency names and passage offsets, paraphrase
   across agencies.
3. Release timing against the portal's own distribution, weekends and court holidays.
4. Hand-imported files labelled apart from agency releases and compared with held documents.
5. Budget under-spending, adopted lines without actuals, operating actuals fetch.
6. Redaction corroboration from imported tables and for older documents when new material arrives.
7. Re-check keeps changed bytes and shows the difference; listing drop-outs; snapshot row and
   column comparison (on the peer's watchlist storage, no second diff store).
8. Docket copies over time with revision differences; settlement resolutions as a table.
9. Draft for an overdue request with no response, citing the agency's counted record.
10. Briefing export with every figure's source; place-level counts; reports after abatements in
    nearby places; a signed receipt for one document.

Built 2026-10-04 (order item 3): `RELEASE_TIMING_CONCENTRATION` (methods §22). Per agency and
portal, the share of releases that began at or after 3 p.m. on the last working day before a
weekend or California court holiday, against the portal's other releases; a release is one
request's files uploaded less than 60 minutes apart. The audit line above is corrected:
`documents.created_at` is the portal's upload time only for a listed file whose portal gave one;
it is the request's opening time, its closing time (closure notices) or the app's first-seen time
on other paths, and those rows are left out or not read. `held_back_response` and
`regenerated_response` time a file by its own upload time when it has one. Measured on
`eval-encampment-2` (142 sandiego.nextrequest.com requests, read-only DuckDB CLI, 2026-10-04; SQL
equivalent of the detector's grouping, weekends only, no holiday calendar): 771 documents, of
which 142 are closure notices and 629 listed files, all with an upload time; 215 releases from
2020-06-18 to 2026-09-10, the largest of 168 files. By weekday: Monday 38, Tuesday 45, Wednesday
35, Thursday 46, Friday 48, Saturday 3, Sunday 0. By starting hour: 5 a.m. 3, 6 a.m. 14, 7 a.m.
17, 8 a.m. 18, 9 a.m. 14, 10 a.m. 19, 11 a.m. 26, noon 19, 1 p.m. 21, 2 p.m. 25, 3 p.m. 26,
4 p.m. 12, 5 p.m. 1. At or after 3 p.m.: 39 (18.1%); on a Friday at or after 3 p.m.: 9 (4.2%).
The agency with most releases has 77, 4 of them Friday from 3 p.m. (5.2%); the next two have 50
and 21, with none and 1; no other agency has 20, so by these counts nothing would be reported
there. Not measured: the
detector itself on that collection (it is opened only by the CLI), and its file ages. That
collection has no `request_release_events` table and no request with a determination time, so the
two older detectors do not run on it.

Built 2026-10-04: a file or pasted text the user adds is no longer read as an agency release.
`documents.visibility` stays `public` (it is the access gate for the viewer, search and table
import, not a statement of origin); origin is answered from `portals.source_kind` through the
`user_supplied_sources` view, which every detector query that selects documents without requiring
an agency or a determination now excludes, and `absence_of_records` reads records releases only.
No stored value changed. The document viewer labels such a file "Added by you. Not downloaded from
an agency source", and `describe_document_origin` compares it with collected documents: same file
SHA-256, same extracted text in a different file, and near-duplicate text from the existing corpus
index, each either compared or not compared with the reason. `read_document_internal_metadata`
shows the PDF Info/XMP or Office docProps fields, each a value or not recorded, stated as written
by the file's maker and unverified. Not built: marking findings drawn from an imported table whose
origin file was hand-added, and excluding hand-added files from redaction-inference corroboration
(architecture doc section 5).

Built 2026-10-05: follow-up letter after a determination deadline passes with no recorded response
(methods §33). For a saved casework request, `draft_overdue_follow_up` states the recorded
submission date, the deadline computed under Government Code § 7922.535 with the court calendar,
and the calendar days since, and asks for the determination; it refuses with a shown reason on the
deadline day and earlier, with no submission date, or with any recorded response. The casework
panel offers it under the request's scheduled follow-ups. After a first draft the user may pick a
library agency (portal plus agency id, at least 10 closed requests) to add one counted paragraph:
closed requests held, sourced determinations and how many of those were after the computed
deadline, and no-records closure notices with a recorded earlier similar release. The lead dossier
now shows a table lead's tables (name, source, retrieval date, SHA-256) and SQL on screen.
Verified by tests: the boundary day and the day after, a valid and a defective extension, a
deadline on 1 January, each refusal, the paragraph at 9, 10 and 11 closed requests, zero numerators
against unmeasured ones, two portals sharing an agency id, the wording check, the stale-result
guard (proved by removing it), and the dossier with and without a table lead. Section 7923.000
was read at leginfo.legislature.ca.gov through a text-extraction tool. Not verified: anything in
the running app; the counts against a real library. Not built: the letter for a tracked request in
My requests (`tracker/deadline.rs` holds the same evaluation and an extension notice, so the
drafting function would accept it; no UI was added there); an extension field on a casework
request, so that path always counts ten days and says no extension is recorded; remembering the
chosen agency on the case; replacing the generic `followUpDraft` text.

Built (meeting bodies, posting time, agenda copies; methods section 27): one registry,
`src/content/meetingBodies.json`, read by both the Rust starter set
(`ingestion/meeting_bodies.rs`, set `city-boards-and-commissions`) and the source catalogue
(`src/utils/meetingBodySources.ts`): 37 bodies, 16 with a verified date in the set, 21 from the
City roster offered singly and marked as not opened. The OnBase adapters accept the committee
application (`211agendaonlinecomm`) and the SDHC attachment rule covers the Housing Authority
page. `document_acquisitions` gained `response_date` (the response `Date` header, NULL for
earlier fetches). The Meetings view shows each file's `Last-Modified` as "posted (per the file's
Last-Modified header)" or not recorded with the reason, the time to a stated meeting start, a
count of agenda files under 72 hours with no conclusion, and item differences between copies of
one agenda held at different addresses. No new command and no new finding kind.
Verified with plain requests on 2026-10-05: the index pages of the Homelessness Revenue Account
Citizens Oversight Committee, Privacy Advisory Board and Planning Commission link agenda PDFs
inside `<main>` under `/sites/default/files/`, which the City preset's rules collect; a committee
meeting page declares the same `ViewAgenda` template as the Council application; sandiego.gov
PDFs carry a `Last-Modified` days older than `Date`, while sandiego.gov HTML and OnBase pages
carry one 0 to 2 seconds before `Date`. Not verified: any collection run in the app; the 21
roster addresses; item links and attachment downloads on the committee application (the one
agenda read was an adjournment with no items); whether SDHC attachment pages send a usable
`Last-Modified`; how each board words its agenda heading, on which body recognition depends.
Not built: comparison of copies published at one address (OnBase dockets, the Civil Service
Commission's fixed agenda file). Re-collection rewrites the one `documents` row and its text, and
a re-check discards the new bytes, so the earlier text is gone; this needs the re-check or
watchlist storage to keep the earlier extracted text per (document, raw SHA-256). Also not built:
SDHC upload times from `wp-json`, the Commission on the Status of Women, Franchise Compliance
Review Committee, Otay Mesa EIFD authority and community planning groups, SDCERS, and a yearly
roll-over of the Ethics Commission's per-year index address.

Round one merged and gated 2026-10-05 (main at 02366f6: fmt, clippy, 1,328 lib tests, 8
data-discovery tests, tsc, eslint, 645 frontend tests, IPC and help checks all exit 0): order
items 1 to 4 above. Nothing in round one has been checked in the running app.

Round two merged and gated 2026-10-05 (fmt, clippy, 1,369 lib tests, 9 data-discovery tests,
tsc, eslint, 666 frontend tests, IPC contract with 48 finding kinds, help coverage, all exit 0):
operating actuals fetch and spending-record leads; the meeting-body registry, posting time and
revision comparison between copies at different addresses; anchored redaction context across
documents and imported tables; the follow-up letter for a request overdue with no response and
a lead's tables and SQL in the dossier. "Built" paragraphs for each are below. Nothing in rounds
one or two has been checked in the running app.

Left open by round two: comparing agenda versions at one address (the earlier text is
overwritten on re-fetch; needs extracted text kept per document and raw SHA-256 in the peer's
re-check or watchlist storage); Housing Commission upload times; five meeting bodies (Status of
Women, Franchise Compliance, Otay Mesa financing authority, community planning groups, retirement
board); redaction sweep timing in an optimised build and on a real corpus.

Next, done in the main session against the one shared cargo target (no parallel worktree builds;
see the owner's feedback on agent cost): disclosure-portal reader and export import
(`docs/data-sources/city-disclosure-filings.md`), settlement resolutions as a table, reports
after abatements in nearby places, a signed receipt for one document, and the data packs in
`docs/data-packs.md`.

Built 2026-10-05: a lead from the disclosure portal's agency-report export (methods §25,
`disclosureLeads.ts`, `DISCLOSURE_FILING_LEAD`): reports per form, and behested payment reports
per official named. Import only; the app makes no request to the portal. Verified on a fixture
with the export's real headers and values. Not verified: a full export (the portal reported 1,333
agency reports, 278 of them Form 803 through 2024, on 2026-10-04), and the lead in the running
app. Not built: the user-started reader for the portal's API, and imports for statements of
economic interests, campaign and lobbyist exports.

Built 2026-10-05: Encampment reports filed around abatements (methods §19 D; `abatementLeads.ts`,
`abatement_reports_nearby`, stored as `ABATEMENT_LOG_LEAD`). Measured with the app's SQL in the
DuckDB CLI on the released completion log (26-772) and the open and closed 2023 to 2026 Get It
Done files retrieved 2026-10-05: 13,387 of 13,406 abatement place-days fall where reports are
complete (2023-01-01 to 2026-10-05; 205,764 Encampment reports, 82 with no coordinates or date).
Report and abatement pairs within 150 metres: 319,347 in the 14 days before, 24,239 on the day,
307,733 in the 14 days after (3.6% fewer after). From 150 to 800 metres: 2,704,069 before and
2,692,389 after (0.4% fewer). So on these counts reports do not rise in the surrounding ring after
an abatement; the pair counts are many times the number of reports because abatements cluster.
The query took 4.5 seconds from CSV. Not verified: its run time inside the app against the
30-second limit, and the lead in the running app. Per abatement, of the 11,208 with at least 10
ring reports beforehand: 5,151 had more afterwards and 5,884 fewer; 809 rose by half or more and
1,114 fell by a third or more. Not built: naming the places where the ring rose.

Measured 2026-10-05, not built (one-off SQL kept at
`/Volumes/A/onrecord-eval/analysis/abatement-reports-deep-2026-10-05.sql`; same files as the
paragraph above, abatements from 2023-07-31 to 2026-01-27):
- Week by week, report and abatement pairs within 150 metres rise into the abatement (148,114 six
  weeks before to 163,240 in the week before) and fall after it (155,784 in the first week to
  142,668 in the sixth). In the 150 to 800 metre ring they stay between 1.34 and 1.36 million
  every week.
- Placebo, the same places with the date moved 28 days earlier: within 150 metres 296,235 then
  304,996 (3.0% more); ring 2,722,122 then 2,707,555 (0.5% fewer). Against that drift the actual
  change is 3.6% fewer at the site and 0.4% fewer in the ring, so the ring shows no change beyond
  the background and the site shows a fall.
- By year of abatement: 2023 (from 31 July) site +0.9%, ring +3.9%; 2024 site -11.2%, ring -6.2%;
  2025 site -1.9%, ring +0.1%. January 2026 (403 abatements) shows site +23% and ring +47%, but
  its "before" fortnight is late December; not to be read without a seasonal baseline.
- Repeats: 131 places with 20 or more abatements account for 7,814 of 13,387 place-days; 2,129
  places were abated once. The most abated coordinates (recorded as 300 16th Street and as 1600 K
  Street) have 331 place-days in two and a half years. One coordinate can carry several address
  strings.
- No place rises consistently: at the most abated place the ring rose by half or more after 24
  abatements and fell by a third or more after 32.
- Of 8,124 abatements with at least 5 reports within 150 metres beforehand, the site count fell by
  a third or more after 2,573 and rose by half or more after 1,430. Site fell while the ring rose
  by half: 139. Site and ring both fell by a third: 235.
- Council District 3 holds 2.44 of the 2.70 million ring pairs.
Built 2026-10-05 from this (methods §19 D and E): the lead on places abated on 20 or more
separate days, and the count taken 28 days earlier shown inside the reports lead. The app's two
statements reproduce the figures above on the real files (131 places, 7,814 of 13,387
place-days; earlier count 296,235 then 304,996 and 2,722,122 then 2,707,555). The reports
statement now takes 12.3 seconds from CSV in the DuckDB CLI; its time inside the app against the
30-second limit is not verified. Not built: the weekly profile.

## Gaps to requests, and organisation filings (2026-10-05)

Decision (owner, 2026-10-05): the app itself, for every user, must (1) surface the record gaps
behind each exploration, try to fill them from public records requests already made, suggest the
precise request to file when it cannot, and track each request through submission, response and
follow-up; and (2) check an organisation's public filings, not have a session read them once.
Nothing here is built yet.

Both must work for other subjects and places, not only San Diego homelessness (owner, same day):
gap definitions are data keyed to what a lead read, not code about one city; the custodian,
portal and statute come from the source the lead's table or document came from; the seed set
below is the first content, not the design. The organisation check takes any EIN.

The owner's five hypotheses, which rank the gaps to seed first (to test, not to assume): police
spending on homelessness enforcement buys little; the City and police do not follow their own
rules on storing belongings and returning them; sweeps accomplish little and make stability
harder; police response to homeless people is outsized beside response to open drug dealing and
prostitution; almost no help exists for people who are not veterans or HIV positive, and shelter
and safe-sleeping rules push people away. Measured so far: reports do not rise around an
abatement (above), so "sweeps move people next door" is not supported; repeat abatement at the
same places is.

### 1. Gap to request chain
Exists: `CaseworkRequest` (gap, custodian, draft, state, submitted_at, response_due, productions,
unresolved_omissions, follow_up_draft, compiled_draft, filing plan), `request_compiler`
(`compile_cpra_request`, `plan_filing`), the overdue follow-up letter, `analysis/gaps.rs`
(`EvidenceGap`, `GapRole`, `GapStatus`, `collect`, `resolve`) for financial roles, and
`dataGaps.ts` (`fetch` and `collect` remedies) for the discovery feed.
To build, in order:
a. A record-gap definition a lead can carry: what record would answer the lead's question, the
   custodian, the period and identifiers from the lead's own result (addresses, dates, activity
   numbers), the fields wanted, and search terms. Seed set: stored versus discarded belongings and
   retrievals (storage site intake and release logs, abatement work orders); abatement cost per
   day and place (contractor invoices); units dispatched per call (CAD unit history) for named
   places and dates; shelter referrals with no bed available; safe-sleeping site rules, incident
   and grievance logs; case outcomes for encampment, encroachment and lodging charges; the
   written procedures themselves.
b. Mine first: search requests already in the library and, through the existing collection
   path, the portal for requests whose text matches the gap; show the matches with their release
   status; offer to collect one. A gap is "answered by a release" only when a released file is
   imported and the lead's check then runs on it, never because a request with similar words exists.
c. Suggest: when nothing is found, a drafted request naming record type, custodian, period,
   identifiers and fields, using the City's SB 272 system catalogue to name the system of record.
d. Track: accepting a suggestion creates the casework request; submission, response, productions
   and follow-ups use what exists. The feed shows each gap's state: not asked, found in a release,
   requested (with days to the deadline), overdue, produced, answered.
e. Close: when a production imports and the check runs, the gap clears and the lead is recomputed.

Built 2026-10-05 (methods §33 F): gap definitions as data with eight seeds (`recordGaps.ts`); the
feed lists each lead's gaps with their state; a user-started search of held requests and of the
table's own portal; the drafted request; tracking as a casework request in a case opened for the
lead (`request_record_gap`); the stored-property check that answers the first gap
(`belongingsLeads.ts`). Not built: a statute other than the CPRA in the compiled request, the
SB 272 catalogue lookup, checks for the other seven definitions, and the stored-property check has
never read a real released log (its column names are assumed).

### 2. Organisation filings
For any tax-exempt organisation the user names (by name or EIN): fetch its Form 990 history and
report, per year, as stated on the filing and nothing more: revenue, expenses, assets, officer
compensation; whether Part VI line 5 reports a significant diversion of assets; current and
former officers and key employees with reportable compensation (Part VII, Schedule J, including
severance); transactions with interested persons (Schedule L); and, where the organisation spent
federal money, its single audits and findings (Federal Audit Clearinghouse, by EIN). Then set
beside it the public money it received from the agencies in the library (contracts, leases,
grants). Sources to verify before building: ProPublica Nonprofit Explorer API v2
(`/nonprofits/api/v2/search.json`, `/organizations/<ein>.json`; summary fields only; terms and
attribution), the IRS e-file XML for the full return (location and index not yet verified),
`api.fac.gov` (free key). Rules: a filing's own answer is quoted as the filing's statement; a
payment to a former officer is reported as a payment, never characterised; an allegation from
any source is not stored as a finding; no finding kind asserts wrongdoing. Verified 2026-10-05:
the summary endpoint returns filing years 2011 to 2023 with those summary fields for one EIN.
Verified later the same day, by request from this machine: the IRS publishes one index per
submission year at `https://apps.irs.gov/pub/epostcard/990/xml/<year>/index_<year>.csv` (2024:
91 MB; columns RETURN_ID, FILING_TYPE, EIN, TAX_PERIOD, SUB_DATE, TAXPAYER_NAME, RETURN_TYPE, DLN,
OBJECT_ID, XML_BATCH_ID) and the returns inside batch archives
`<year>/<XML_BATCH_ID>.zip` (`2024_TEOS_XML_01A.zip`: 105 MB, `accept-ranges: bytes`), so one
return can be read by ranged requests for the archive's directory and one member rather than the
whole archive (a ranged request returned 206; members are named
`<XML_BATCH_ID>/<OBJECT_ID>_public.xml`). ProPublica's
`/nonprofits/download-xml?object_id=` answers 403 with a browser security check and is not to be
used. `api.fac.gov/general?auditee_ein=eq.<ein>` answers 403 without an `X-Api-Key`. Open design
question: an EIN's history needs every year's index (about 90 MB each); fetch them only when the
user asks for an organisation, and keep only the rows for EINs asked about.

Decided and built 2026-10-05 (methods §34): an index is fetched when a check needs a year the
library does not hold (or holds a copy older than seven days, for the current and previous year),
and every row is kept in `irs_efile_index`, so the next organisation costs no download (a first
design kept only the checked EIN's rows; a programme analyst monitoring a portfolio of providers
would have paid 90 MB a year per provider). Returns are read by byte range (`remote_zip.rs`,
`ApiClient::get_direct`), parsed with `roxmltree` (`parsers/form990.rs`, element names taken
from 9,681 published returns), and stored with their XML and SHA-256. Commands
`search_organisations`, `check_organisation_filings`, `get_organisation_filings`,
`list_organisation_checks`; view "Organisation filings" in the Investigation workbench; ProPublica
summary with attribution; single audits with a user-supplied key; vendor-name matches from the
mapped contract, amendment and payment tables. Found while building: the 2017 to 2023 indexes
have no `XML_BATCH_ID`; the 2024 index writes three archive names in lower case that the server
only serves in upper case; 2024 archives put members under a directory and 2025 and 2026
archives do not, and compress with Deflate64 (crate `deflate64`); archives other than `01A` are about 1 GB with a 15 MB directory and
need the zip64 record; `api.fac.gov` answers with an api.data.gov key (`DEMO_KEY` worked).
Measured by the ignored live check (`live_organisation_check`, EIN 33-0215585): three returns read
(tax periods 2023-06, 2024-06, 2025-06; revenue 41,418,753, 46,119,889 and 41,585,915 as stated),
13 summary years (2011 to 2023), ten single audits (2016 to 2025), and a second check that
refetched no return and returned the same record.
Also built the same day for that use: a payee-table mapping (any imported table's vendor, amount
and fiscal-year columns) searched beside the spending-role tables, and a one-row-per-organisation
portfolio table over everything checked, and a pasted list of names or EINs (or the payees of the mapped payment tables) looked up and
checked one after another; a name is taken on its own only on an exact match.
Not built: returns from index years before 2024 (no archive named; would need every archive's
directory for a year, about 150 MB); Forms 990-EZ, 990-PF and 990-T; Schedule R; single-audit finding text; saving the clearinghouse key (it is
typed each session); payments matched by anything but vendor name; progress reporting during a check; watchlist or packet integration
(peer-owned). Not checked in the running app: the view has only been exercised by its tests.

## Leads from service-report tables (2026-10-04)

Decision: "What stands out" shows leads about the subject of the records; facts about a table
(latest period, record shares, gaps, repairs) are folded beneath them. An earlier pass that added
Pearson correlations, IQR outlier counts and percent-change "spikes" to the feed was reverted:
they describe the table, the correlation SQL did not parse, and an unused cross-join "proximity"
module was quadratic. Method: `docs/analysis_methods.md` §17.

Measured on the owner's library (Get It Done open file retrieved 2026-09-14; read-only DuckDB CLI
against the app database, same SQL the app generates):
- 42,322 of 83,032 open reports with a service type are older than a year. Sidewalk Repair Issue
  11,929 of 14,690 (earliest 2016-05-22); Street Light Maintenance 8,046 of 14,358; ROW
  Maintenance 6,166 of 10,564.
- Open street-light reports: median 881 days old in Peninsula and 867 in Pacific Beach, against
  462 across all areas. Peninsula street lights also closed slower than the all-area median in
  each of the closed-2016, 2017 and 2018 files (37 vs 23, 85 vs 27, 88 vs 40 days).
- 78 open street-light reports name Mission Bay Bike Path (earliest 2021-09-22); 26 name 9711
  Oviedo Way (earliest 2022-04-28).
- Referred with no outcome in the table: 56,979 of 139,362 reports in closed-2017, 39,537 of
  169,591 in closed-2018.

"Fetch and import" now offers the citywide open file (`GET_IT_DONE_OPEN`, 31 MB, republished
daily). Next, in order:
1. Done 2026-10-04: reports that left the open file between two copies, and new arrivals, per
   service, with their close or referral record (`service_movement`). A second import of one
   source URI keeps the earlier snapshot (exercised in `tests/data_discovery.rs`). Measured with
   the app's SQL on the 2026-09-14 copy against the file published 2026-10-04 and the closed-2026
   file: 8,016 reports with a service type left and 8,939 arrived; of those that left, 7,832 have
   a close date, 184 were referred, none lacks a record; 657 had been open more than a year (ROW
   Maintenance 170, Street Light Maintenance 95). Closures of long-open reports are spread across
   days (at most 78 on one day), not a bulk close-out.
   Decision (owner, 2026-10-04): when the imported tables cannot answer a lead's question, the
   app fetches the table that can. Built for Get It Done (`src/utils/dataGaps.ts`, methods §17 C):
   a newer open copy after seven days, and the closed-reports year filtered to reports seen open.
   Not built: seeking a source for a gap outside that registry (searching the source catalogs in
   `src/utils/*Sources.ts` or the publisher's dataset index by the missing role).
2. Budget half done 2026-10-04: the backlog lead names the publisher's staff group, the budget
   departments it is read as, and their adopted operating expense (methods §17 B "Owner and
   budget"); a missing budget table is fetched as a gap. `case_record_type` proved to be a staff
   group label, not a department code. Measured on the 2026-09-14 open file and the budget file
   published 2026-10-04: Sidewalk Repair Issue 14,645 of 14,690, Street Light Maintenance 14,332
   of 14,358 and ROW Maintenance 10,471 of 10,564 open reports are under TSW; adopted expense
   Transportation 190.6 million (FY 25) and 172.8 million (FY 24), Stormwater 73.9 and 67.6.
   Not built: the department total is not the service's line (the budget file has no program
   below funds center; `budget_lines.rs` is not used), and agenda matching: `research::meetings`
   matches findings by portal agency, and these findings name a department, not a portal agency.
3. Done 2026-10-04: service leads are stored as `SERVICE_REPORT_LEAD` findings
   (`record_service_leads`, methods §17 E). They carry no portal agency, so agenda suggestions
   (§11 "Stored finding subject") do not match them until item 2 names the owning department.
4. The same role-based checks for code-enforcement cases and permits (opened, closed, type, area).
5. Full Get It Done closed years exceed the 96 MB import limit; only Encampment rows are kept.
   Area comparisons for other services need a per-service subset or a streamed aggregate.
