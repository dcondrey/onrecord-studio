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

1. `no_records_contradicted` by meaning, not word overlap. 13/140 requests closed "No responsive
   documents" (closure notices are stored as documents). Word overlap matched them to unrelated
   releases under every token rule tried (all-8, 75%, extra stopwords); match the denied request's
   text to releases with the on-device embedding model, and require the release to predate the
   denial's closure.
2. Rolling productions: 24/140 requests released in installments, 6 over 180 days (20-2226: 30
   releases over 809 days). Needs release events stored per request (timeline "Document(s)
   Released" bylines); a pattern finding, not a legal claim; never derive `determination_at`.
3. Done: disposition counts per department (`analysis/dispositions.rs`,
   `get_department_dispositions`, table under Compare agencies). Classified from the stored
   closure notice by ordered word-phrase rules; "not recorded" (no notice) and "not classified"
   (notice no rule matches) are separate counts. On the same collection (142 requests, re-read
   2026-10-03): 88 released, 35 released in part, 3 withheld, 13 no records, 3 withdrawn, 0
   unclassified, 0 unrecorded. The earlier 87/33 figures came from a different run. Rules are
   measured only against San Diego's wording; other NextRequest portals' notices will show as
   not classified until a rule names them.
