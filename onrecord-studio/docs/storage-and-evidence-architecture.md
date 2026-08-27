# Storage and Evidence Architecture

What is canonical, what is rebuildable, where SQLite is still allowed to appear, and how strong a
provenance result actually is. Written because three names in this tree describe something other
than what they are, and the gaps between name and behavior have produced real defects.

## 1. The core is DuckDB

`storage::sqlite::DuckDbRepository` owns the single durable writer and every canonical table:
portals, agencies, requests, documents, case audits, statutory extensions, legal authorities,
statutory violations, provenance manifests, and the provenance verification log. Writes are
serialized through one `Arc<Mutex<Connection>>`; analytics reads run on sibling connections to the
same instance.

Two names survive for compatibility and neither means SQLite is involved:

| name | reality |
|---|---|
| module `storage::sqlite` | the DuckDB core. Renaming the file would touch command and ingestion modules that do not otherwise change. |
| `storage::sqlite::SqliteRepository` | a type alias for `DuckDbRepository`. |

`SqliteRepository` must stay a **type alias**, never a newtype. Tauri resolves `State<T>` by
`TypeId`, so the alias makes `State<SqliteRepository>` and `State<DuckDbRepository>` the same type
and lets modules mix the two freely. A wrapper struct would compile and then fail at runtime on
state lookup. Prefer `DuckDbRepository` in new code; both are exported from `storage`.

## 2. Derived projections

Everything below is rebuildable from the core. None of it is evidence in its own right.

**`documents_fts` — search projection.** A plain DuckDB table maintained transactionally by the
repository inside the same transaction that finalizes a document extraction. **It is not an FTS5
virtual table**: no tokenization, no ranking, no Boolean operators.
`DuckDbRepository::full_text_search` reduces a query to a single term and matches it with a bounded
`LIKE` substring scan. A mid-word fragment matches; `AND`/`OR`/`NOT` are dropped.

`storage::fts::format_sqlite_fts5_query` emits real SQLite FTS5 `MATCH` syntax and is tested
against a real FTS5 virtual table, but **no core path calls it**. It is retained because it is the
piece a real Boolean search needs, and because the SQLite attachment boundary can still present
FTS5 tables. `format_fts5_query` remains as an alias of the qualified name.

**Corpus index** (`storage::corpus`). Reconciled from the core at startup by a keyset cursor.
Failures are reported without blocking startup, because a projection that is behind is a
degradation and a core that will not open is an outage.

## 3. Where SQLite is still allowed

Only at boundaries, never in the core:

- **Legacy import** (`storage::legacy_import`) upgrades a pre-DuckDB core written by an earlier
  release. DuckDB cannot read a SQLite file, so without this an old installation fails at startup.
- **Foreign attachment.** `DuckDbWorkspace` and `analysis::anomaly::AnalyticsEngine` can `ATTACH` a
  read-only SQLite file an investigator supplies, so it can be analyzed without being imported.

Both analytics nodes have two modes and they are not interchangeable. `bind_to_core` takes a live
sibling connection; `ATTACH`ing the core file instead would pin the node to the database as it
stood at launch, so nothing ingested during the session would ever be analyzed. The `Core` mode
therefore rejects attachment outright.

## 4. Statutory compliance has one model

`analysis::statute` is the single definition of CPRA determination timing. Two paths consume it:

- `analysis::timeline` evaluates one request in Rust, for briefs and audit refresh.
- `analysis::anomaly` evaluates every request at once inside DuckDB, for agency aggregates.

The aggregate cannot call the Rust evaluator row by row without giving up vectorized execution, so
its SQL is *generated* from `statute::sql`. Thresholds, the determination proxy, the
extension-validity rule, and the calendar-day derivation each have exactly one definition.
`statutory_verdicts_match_the_per_request_evaluator` drives both renderings from one table of cases
and fails if either changes alone.

**Instants versus calendar days.** Durations and ordering use `TIMESTAMPTZ`-derived instants.
Every day count uses the RFC 3339 date prefix, which is the date in the offset the agency recorded.
Converting an instant into the session time zone before counting days makes the verdict depend on
where the application runs: a request received at `2026-01-01T23:00:00-08:00` counts from 1 January
in Pacific and from 2 January in UTC, which is the difference between a violation and a clean
record. The parity test runs the aggregate under three session time zones and requires identical
results.

**The determination is a proxy.** The statute runs to the agency's determination, a distinct noticed
event that no ingested portal models. Both paths substitute the earliest recorded activity at or
after receipt (`updated_at`, else `closed_at`) and report which substitute they used in
`StatutoryComplianceAnalysis::determination_basis`:

| basis | meaning |
|---|---|
| `EARLIEST_RECORDED_ACTIVITY_PROXY` | measured to recorded activity, which is an upper bound on when the agency acted, not evidence that the act was a determination |
| `OBSERVATION_WITHOUT_RECORDED_ACTIVITY` | nothing recorded after receipt; the period is still running and the finding is provisional |

Do not present a finding without reading the basis.

**Two separate claims.** `AgencyPerformanceProfile::compliance_indicator` reports
`StatutoryDeadlineExceeded` only when a statutory violation was counted, which is a legal
conclusion drawn from the statute's calendar-day rule. An agency whose latency z-score crosses
the threshold with no counted violation reports `LatencyOutlier`, which is a statistical statement
about its peers that no statute speaks to. An agency that is both reports the statutory verdict,
the stronger claim. `latency_outlier_without_a_violation_is_not_labeled_statutory` pins the
separation. `LatencyOutlier` is computed only; it is never written to `case_audits`.

## 5. Provenance confidence

`ProvenanceConfidence` separates what this application **recorded** from what it **verified**. The
distinction is the point: an investigator citing a document in a filing needs to know which one
they are holding.

| level | evidence behind it |
|---|---|
| `DIGEST_MISMATCH` | stored bytes do not hash to the recorded digest |
| `NO_RECORDED_DIGEST` | nothing recorded to compare against |
| `RECORDED_CLAIM_ONLY` | manifest metadata on file, no digest to check it against |
| `LOCAL_DIGEST_MATCH` | stored bytes hash to the recorded digest |
| `VERIFIED_SIGNATURE` | a signature was cryptographically verified over the asset |
| `TRUSTED_CHAIN_OF_CUSTODY` | a verified signature chains to an independently trusted anchor — **this build cannot produce it** |

`VERIFIED_SIGNATURE` is the ceiling. It is reached, end to end, by any asset carrying a manifest
whose signature verifies over its hard binding;
`a_signed_asset_verifies_and_a_tampered_one_reports_an_invalid_manifest` signs a fixture with a
throwaway certificate chain and reads it back through `read_asset_provenance` to prove it, and
proves the tampered copy reads `Invalid`.

Almost every released record carries no manifest at all, so `LOCAL_DIGEST_MATCH` is what most
documents reach. That is local integrity, not provenance: it shows the file has not changed since
this application wrote it down, and says nothing about who produced it or whether any attached
manifest is genuine. `recorded_digest_source` records whether the compared
digest came from a manifest (a third party's statement) or from the ingestion record (this
application's own note).

### C2PA manifests are read from the asset, live

Manifests are no longer read from storage. Each verification opens the gated evidence file with the
C2PA SDK and validates it, because a stored row describes what was true when it was written and a
verification has to describe the file in front of it. A manifest row recorded earlier is replaced
by what the asset carries now; a stale row cannot make a document without a manifest report as
having one.

The SDK's `ValidationState` maps onto the ladder directly:

| ValidationState | level | meaning |
|---|---|---|
| `Invalid` | never raises the level | a manifest is present and its integrity check failed |
| `Valid` | `VERIFIED_SIGNATURE` | the signature verifies over the asset's hard binding |
| `Trusted` | `TRUSTED_CHAIN_OF_CUSTODY` | the signing certificate chains to a configured trust anchor |

**`Trusted` cannot be produced by this build.** The SDK verifies against a trust list but ships an
empty anchor set, and nothing in this application supplies one or offers anywhere to configure one,
so validation tops out at `Valid`. That is a deployment fact, not a property of a document, and a
`Valid` result therefore carries an explicit finding saying the signer's identity was not checked.

**Decision: trust anchors will be configurable, and the rung stays.** It needs a settings surface
and a `Settings.trust` anchor list passed through the `Context` built in
`provenance::read_asset_provenance`, not a change to the ladder or to any consumer.
`TRUSTED_CHAIN_OF_CUSTODY` is the only level that is independent third-party evidence rather than
this application's own bookkeeping, which is precisely what an investigator citing a document in a
filing needs, so removing it would mean removing the one rung worth reaching. Until that surface
exists, every doc comment and badge says the level is unreachable in this build rather than
implying an operator can already switch it on. To reverse the decision, delete the
`TrustedChainOfCustody` variant and its TypeScript counterpart, drop `PROVENANCE_BADGES` entry, and
map `ValidationState::Trusted` onto `VERIFIED_SIGNATURE`.

**A digest mismatch dominates any manifest verdict.** A validly signed manifest says the asset
matches *its own* manifest, which a substituted file carrying its own valid manifest also
satisfies. Only the digest recorded at ingestion says the file is the one this application
downloaded, so a mismatch is reported as a substitution however good the manifest looks. An invalid
manifest, conversely, never lowers the level below what the digest earned — a broken provenance
claim does not make an intact file less intact — but it always produces a finding.

**Verification reads local bytes only.** The `fetch_remote_manifests` feature is deliberately off.
With it on, validating an asset follows a URL embedded in that asset, so examining a document would
send a request to a server the document's author chose: both a disclosure of what an investigator
is looking at and an input the subject of the investigation controls.

The SDK returns an error both for an asset with no provenance and for one whose provenance will not
parse. Absence is the default, because reporting "manifest present but invalid" on every
unrecognised file would put a broken-claim finding on most of a release. A manifest is only ever
reported invalid when the SDK actually parsed one; an unreadable asset is surfaced as a finding
without inventing a manifest.

### Two independent checks

`verify_document_provenance` reports on two things, and neither verdict stands in for the other.

**The asset**, via `provenance_confidence` above: do the bytes on disk still hash to the digest
recorded for them.

**The derived text**, via `text_projection_matches`: does the text in `documents_fts` still hash to
`documents.extracted_text_projection_sha256`. This matters because `extracted_text_sha256` covers
the *raw* extracted text, which is discarded after finalization -- it is checkable at write time
and never again. The normalized projection is what search returns and what a legal brief quotes,
and it had no digest at all until one was recorded alongside it. A document can be byte-identical
to what was ingested while the text quoted from it has drifted, so a drift verdict must never move
the asset verdict, and `None` means the comparison could not be made rather than that it passed.

### Reading evidence

The evidence path is resolved from the document row and gated by
`commands::validation::readable_evidence_file` before anything is hashed: symbolic links are
refused and the resolved path must stay inside the managed evidence store. Unlike
`managed_evidence_file`, the gate accepts empty files, applies no size ceiling, and reports refusal
as data rather than as an error, because a verification that cannot read its file still has to be
recorded. `EVIDENCE_FILE_UNAVAILABLE` carries the reason and is appended to the log like any other
outcome; previously this case raised an error, so the one fact an investigator most needs on the
record -- that the file was gone when they checked -- was the one fact the log never captured.

A recorded digest that is not well-formed hex is reported as a corrupt record, not as a mismatch.
Comparing against it would fail and accuse an intact file of having been substituted.

### The verification log

`provenance_verification_log` rows are the only ones describing work actually performed. The log is
append-only by repository API contract: DuckDB has no triggers, so this holds because no mutation
path exists in Rust, not because the schema forbids one.

A row records the whole result, so a verification can be reconstructed from it rather than merely
summarized. `list_document_provenance_verifications` reads a document's history newest-first; until
it existed the log was written and never read, so the history an investigator would cite in a
filing was unreachable from the application that produced it.

Rows written before the classification columns existed carry the `UNCLASSIFIED_LEGACY_RECORD`
sentinel and read back with `predatesClassification: true` and `null` classifications. That is not
the same as a recorded low-confidence result and must never be displayed as one.

The manifest and log writes for one verification share a transaction, so the log can never point at
manifest state that was not committed.

## 6. Migration rules

- `0001_initial_schema.sql` is idempotent and re-executed on **every** launch. Every statement is
  `CREATE ... IF NOT EXISTS` or an equivalent guarded form.
- Columns added after the fact go through `ensure_column`, which is a no-op when the column exists.
- **DuckDB cannot `ALTER TABLE ... ADD COLUMN` with any constraint attached** — it fails with
  `Adding columns with constraints not yet supported`. A `NOT NULL`, `CHECK`, or constrained
  definition passed to `ensure_column` parses fine on a fresh database, where the base schema
  already created the column and the call is a no-op, and then fails at startup on exactly the
  installations the shim exists to upgrade. Use `DEFAULT` alone, and give the base schema the same
  unconstrained shape so a fresh database and an upgraded one are identical.
  `migrations_are_idempotent_and_persist_provenance_classification` covers this by dropping the
  columns from a non-empty table and re-running the migration.
- A default that stands in for information that never existed must be a sentinel, not a plausible
  value. Provenance rows written before classification existed carry
  `UNCLASSIFIED_LEGACY_RECORD`, because their confidence cannot be reconstructed after the fact —
  the file they described may since have changed.
- An index blocks `DROP COLUMN` on its table but **not** `ADD COLUMN`, so `ensure_column` stays safe
  against an indexed table. This was measured, not assumed, and is pinned by
  `migrations_are_idempotent_and_persist_provenance_classification` — which has to drop the log
  index before it can reproduce a pre-classification shape.
- A pre-DuckDB core is migrated by `legacy_import` **before** anything opens it, and the original is
  retained as a backup rather than replaced.
- Referential integrity is enforced by repository write paths where DuckDB has no cascading foreign
  key. Removing a physical constraint means adding a write-path check, not dropping the invariant.

## 7. Source lifecycle

A saved source is `ACTIVE`, `PAUSED`, or `REMOVED`, stored on `portals.lifecycle_state`.

**It has to be durable.** Halting a source used to live only in the in-memory pool map. The next
launch ran `recover_interrupted_source_keys`, found the source's rows still in
`ingestion_task_frontier`, read them as an interrupted crawl, and resumed a source the operator had
deliberately stopped. Startup recovery now joins `portals` and considers `ACTIVE` sources only.
`a_halted_source_survives_the_upgrade_and_is_not_resumed_at_startup` builds the pre-lifecycle
`portals` shape directly and pins both the upgrade and the guard.

**A pause keeps the frontier; a removal discards it.** That is the entire difference between a
resume that continues and one that re-crawls from the first page. Adoption is by
`adopt_orphaned_frontier_tasks`, whose predicate is "no run that is still `RUNNING` holds this
work" rather than any particular terminal status: a crash leaves `INTERRUPTED` and an operator stop
leaves `CANCELLED`, and the queued work each abandons is equally resumable. Matching `INTERRUPTED`
alone stranded every stopped source's rows, which were then never adopted and never deleted, so the
source stayed in the recovery list forever while each relaunch started over.
`a_cancelled_runs_frontier_is_adopted_by_the_run_that_replaces_it` pins it.

**Removal is a state, not a `DELETE`.** The `portals` row is the parent of `requests`, `documents`,
`case_audits`, `statutory_violations` and six more tables. Deleting it is a corpus purge, so
removing a source withdraws it from the listing, discards its frontier and its learned connector
recipe, and retains every document ingested from it. `SourceRemoval` carries the retained document
count and the confirmation dialog states it, because a "Remove" that silently keeps hundreds of
records and one that silently destroys them are both false statements if the interface does not
distinguish them.

**Provisioning a pool is what makes a source active.** `insert_or_update_portal_registration` sets
`lifecycle_state = 'ACTIVE'` on conflict, which is the single funnel every start path reaches. Left
out, a re-added source stayed `REMOVED`, stayed filtered out of the listing, and re-adding it
appeared to do nothing at all, permanently.

**The marker is not part of source identity.** `PortalRegistration` is compared for equality to
decide whether a portal key still names the same verified endpoint, and a mismatch raises a
security refusal. Lifecycle state is carried beside it in `SavedSource`, never inside it, so a
pause can never present itself as an endpoint that changed.

## 8. Follow-up

- Model an explicit, sourced determination event in ingestion and promote both statutory paths off
  the activity proxy.
- Give the core a persistent DuckDB token index and route `full_text_search` through a sanitizer
  written for it, so the query language the frontend offers is the one the core executes.
- Configure C2PA trust anchors. Without them validation stops at `VERIFIED_SIGNATURE` and no
  document can reach `TRUSTED_CHAIN_OF_CUSTODY`, which is the only level that is independent
  third-party evidence.
- Validate provenance during ingestion, not only on demand. Manifests are currently read when a
  document is verified, so a document nobody has verified has no recorded manifest state.
- Add a signed C2PA fixture to the test suite. The confidence rule is covered exhaustively as a
  pure function, and the reader is covered for assets without provenance and for unparseable ones,
  but no test exercises a genuinely signed asset end to end.
- Derive `statutory_violations.confidence_basis_points` from `determination_basis`. Every finding is
  persisted at 10000 (100%), including one measured to the observation instant because nothing was
  recorded after receipt. The `evidence_summary` prose discloses the basis, so this is not a silent
  mislabel, but a provisional finding exported to a legal brief should not carry the same
  confidence as a completed one. This is the compliance analogue of section 5.
