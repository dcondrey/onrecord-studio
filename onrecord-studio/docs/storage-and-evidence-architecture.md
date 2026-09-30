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

## 8. Signed chain-of-custody export

`export_legal_brief` and `export_case_evidence_package` (`src-tauri/src/commands/export.rs`) both
produce a `SignedPackage<T>` (`src-tauri/src/integrity.rs`): a manifest, the exact canonical bytes
that were hashed and Ed25519-signed, and the signature. A verifier never re-serializes `manifest`
to check the signature — it checks the embedded canonical bytes directly — because two JSON
encoders are not guaranteed to produce identical bytes, and a chain-of-custody proof cannot depend
on that agreement. `scripts/verify-evidence-package.mjs` verifies a package with no Rust toolchain
and no trust in this application's own code, which is the actual point of a signature: a party who
does not trust OnRecord Studio's export can still check what it produced.

The signing key (`.onrecord-export-signing-key-v1`, alongside the report directory) is
self-managed and local. Every package's `signature.identityAssurance` says so verbatim
(`SELF_MANAGED_LOCAL_KEY_NOT_EXTERNALLY_ATTESTED`) so a reader cannot mistake it for a notarized or
third-party-attested identity — the same distinction section 5 draws between `VERIFIED_SIGNATURE`
and `TRUSTED_CHAIN_OF_CUSTODY` on the ingest side. The signature proves the exported bytes have not
changed since this installation produced them; it never upgrades an `UNVERIFIED_PORTABLE` or
`HISTORICAL_REPORT` citation into something proven, and a case export lists every such citation by
name in `unverifiedCitationLabels` rather than folding it into a single verified/unverified bit.
Neither export command claims the result is admissible in any court; that is a legal determination
this software cannot make.

`export_casework_packet` (`src-tauri/src/commands/casework.rs`) signs a portable case packet with
the same local key, wrapping it in the same `SignedPackage<CaseworkPacket>` and reusing
`verify_evidence_package`/`scripts/verify-evidence-package.mjs` unchanged — a case packet is just
another `SignedPackage`, not a parallel format. `preview_casework_import` and
`import_casework_packet` verify that signature before the repository ever sees the packet; a packet
that is unsigned or was edited after signing is rejected outright rather than previewed with a
warning. This is a breaking change: a packet exported before this signing was added has no
signature and is now refused. Signing narrows, but does not close, the gap the repository's
`contributor` warning names: it proves a packet's bytes are unaltered and came from whoever holds
one specific installation's key — so two packets claiming the same contributor either share a key or
one of the claims is false — but the key still identifies an installation, not a notarized
real-world person. `CaseworkImportPreview.signerKeyId` and the imported case's event history both
record which key produced an import, so that check survives past the review that first ran it.

### The local export ledger

`export_case_evidence_package_signed` (`src-tauri/src/storage/casework.rs`) reserves the next
position in a per-case, append-only ledger (`casework_export_ledger`) inside the same transaction
that reads the ledger's head and signs the manifest, so two exports racing for the same case can
never fork it — the shared connection mutex behind `query_connection` serializes them, and
`PRIMARY KEY(case_id,sequence)` is the second, independent guard if that assumption ever stops
holding. Every manifest carries `ledgerSequence` and `previousExportSha256` (explicitly `null`,
never omitted, only for sequence 1): a reader chains one export to the one before it by comparing
`previousExportSha256` against the prior export's own `manifestSha256`. The chain is scoped per
case, not global, on purpose — a recipient holding only one case's exports must be able to verify
every link without a global chain forcing disclosure of unrelated cases, and a global chain would
also have pulled in legal briefs and casework packets, which carry a different manifest shape and
were never part of this design. Casework packets (`export_casework_packet`) are deliberately
**not** part of this chain; they are a separate, independently signed and verified artifact
(section 8 above).

Reserving a sequence and durably writing the exported file are two different operations that can
fail independently, so the ledger records their outcomes separately rather than conflating them:
`casework_export_outcomes` is itself append-only (never `UPDATE`d), and a sequence with a
reservation but no recorded outcome reads as `RESERVED` — "signed and chained, but never confirmed
written" — not as `WRITTEN` or silently dropped. `case_export_chain` (surfaced in the UI's "View
export chain") computes a `previousLinkValid` verdict per entry by comparing each entry's
`previousManifestSha256`
against its immediate predecessor's `manifestSha256`, so a reader gets a stated PASS/FAIL, not raw
hex strings to eyeball. That in-app verdict is checked against this installation's own local
`casework_export_ledger` table, which anyone with write access to the DuckDB file can edit; it
proves the ledger is *internally consistent*, not that it has not been rewritten wholesale. The
independently checkable claim is the one anchored in the signed files themselves — the standalone
script's multi-file chain check, which only trusts bytes it can verify against an Ed25519
signature, is the authoritative check for a party that does not trust this installation.

`casework_events.payload_zstd` (one full compressed case snapshot per revision, forever) was
removed as a write path: nothing in this application ever read it back, so it was pure storage
growth with no reader. It is not dropped from the table — DuckDB's `DROP COLUMN` on an upgraded
installation would destroy every existing user's already-recorded history irreversibly, which was
never authorized — so the column stays and every new row writes an empty (zero-length) blob into
it. An empty byte string is not valid zstd; `encode`/`decode` (`storage/casework.rs`) never round
trip it as a value, so nothing can misread the empty placeholder as "an empty case." `author`,
`summary`, and `recordedAt`, which the UI's contribution trail actually reads, are unaffected.
`casework_cases.payload_zstd` (the current-state cache) and `casework_imports.payload_zstd` (full
historical packets from external imports) are unrelated and unchanged.

### Redaction-aware export

`export_case_evidence_package`'s `withheldCitations` parameter (`{citationId, reason}[]`) removes
the named citations from the manifest's `case.evidence` array and strips their ids from every
citation-id list elsewhere in the case (questions, requests, productions, decisions, commitments),
so the manifest never leaves a dangling reference to evidence it omits. `redact_case_for_export`
requires a non-empty, non-duplicate reason per withheld id and rejects an id that does not resolve
to a real citation — an export never ships a best-effort redaction. Each withheld citation is
still named by label and reason inside the signed manifest's `withheldCitations` field, never
silently dropped, and is excluded from both `verifiedCitationCount` and
`unverifiedCitationLabels` — a withheld citation is not counted as anything.

### Key rotation and revocation

`rotate_signing_key` (`src-tauri/src/integrity.rs`) replaces this installation's active signing
key and records the retired key's id in a local, signed revocation list
(`.onrecord-export-signing-key-revocations-v1.json`, alongside the report directory, itself a
`SignedPackage<SigningKeyRevocations>` signed by the *new* key). The retired key file is archived
under a timestamped name via `hard_link`, never deleted and never reached by `rename`-ing the
active key path away: `key_path` is only ever replaced by an atomic `rename` of a fully-staged
replacement, so a concurrent `load_or_create_signing_key` call (or a mid-rotation crash) can never
observe the active key file as briefly missing. The new revocation list is built, signed, and
durably written (temp file + `rename`) *before* the old key is archived or the new key activated,
so a failure at any point up to that write leaves the old key still fully active and the retired
key nowhere recorded — never the reverse, where a compromise could be lost because the key was
already rotated. The new key file is written with the same `0o600`-and-`fsync` discipline
`load_or_create_signing_key` uses for a fresh key; it is never left at default (world-readable)
permissions.

`rotate_signing_key` and `mark_key_compromised` both read-modify-write this list, and the UI's
shared `busy`/request-gate state does not actually serialize them the way it looks like it should:
`open()` (switching the selected case) resets `busy` to `false` regardless of what is still in
flight, so a user can re-enable and click "Rotate key" while a "mark compromised" write from before
the switch is still pending. Two such calls reading the list before either has written it back would
let the second write silently discard the first call's entry — including a COMPROMISED entry a
concurrent rotation raced out, which is the same severity-downgrade this document treats as a defect
everywhere else. A process-wide `tokio::sync::Mutex<()>` (`integrity::REVOCATION_WRITE_LOCK`) now
serializes every read-modify-write of this list within one process, held across the load, sign, and
durable-write `.await` points of both functions; a regression test
(`concurrent_mark_compromised_and_rotate_do_not_lose_either_entrys_write`) runs both concurrently on
a multi-thread runtime and was proven against the actual race by temporarily removing the lock. This
in-process lock is layered under `acquire_cross_process_revocation_lock`, an OS-level advisory
`flock`/`LockFileEx` on a dedicated `.onrecord-export-signing-key-revocations-v1.lock` file (not the
revocations file itself, since that file is replaced via `rename` on every write and a lock on an
inode a write discards would stop protecting anything after the first write). Both functions hold
both locks for the same critical section, so two separate app instances — or processes — writing the
same report directory are now serialized too, not only two calls within one process.

Every entry carries a `status`: `RETIRED` (routine hygiene rotation — packages the old key already
signed remain trusted; shown for information only) or `COMPROMISED` (the key itself is no longer
trusted, and every package it ever signed, past or future, must fail verification). All three
verification paths — `verify_evidence_package`, `verify_and_unwrap_packet`, and the standalone
`.mjs` script's `--revocations` flag — share this distinction via `integrity::check_revocation`
and fail closed on it: only an exact `"RETIRED"` status is treated as informational, and anything
else (a typo, a future status value, corrupted data) reads as compromised, never as silently
trusted. The `.mjs` script cannot read this installation's local state, so `--revocations` takes a
copy of the distributed list file, verifies the list's own signature and that its `entries` came
from the verified canonical bytes (not the adjacent unsigned `manifest` field, which an attacker
could edit without touching the signed bytes) before trusting a single entry in it, and rejects a
file that is unsigned, tampered, or not actually a revocation list (its `schemaVersion` must be
`onrecord-signing-key-revocations-v1`) rather than silently treating it as absent. `casework_imports`
gained a structured `signerKeyId` column (upgrade-safe `ALTER TABLE ADD COLUMN`, type only, per
CLAUDE.md's DuckDB constraint rules): `compromised_case_imports` (`commands/casework.rs`)
cross-references it against the revocation list whenever a case is opened, and flags any already-
imported revision whose signer was later marked compromised — the case content itself may still be
accurate; the warning says only that the channel it arrived through is no longer trusted.

**Recovering from a corrupt or unreadable revocation list**: `load_revocations` treats a missing
file as an empty list (an installation that has never rotated its key), but a file that exists and
fails to parse or verify is a hard error, and every verification and casework import fails closed
while it persists — this is deliberate (a torn or tampered revocation list must never be silently
treated as "no revocations"). The only safe recovery is restoring a valid copy of
`.onrecord-export-signing-key-revocations-v1.json` — from a backup, or from a copy already
distributed to a recipient via `--revocations`. **Do not delete it as a recovery step.** Deleting it
resets the list to empty, which makes every previously COMPROMISED key read as never revoked.
`integrity::mark_key_compromised` (exposed as `mark_signing_key_compromised`, with a "Mark
compromised…" action per entry in the rotation UI) escalates a key this installation has *already*
recorded — typically a routinely `RETIRED` key later found to be compromised — to `COMPROMISED`
without another rotation; it is not a rebuild tool, since it requires an existing entry for that
`key_id` and refuses to invent one. A wholly deleted or unrecoverable list is still a known gap:
there is no path to reconstruct entries this installation never wrote down elsewhere, so the same
backup/redistribution recovery above remains the only option. Note also that the revocation list's
signature only proves *a* key this installation once held signed it — a recipient checking
`--revocations` is trusting the list's signer key implicitly, not verifying it is bound to a
specific installation identity; comparing the signer key id against one obtained through a separate
trusted channel is the recipient's responsibility, the same as for any manifest signer.

## 9. Follow-up

- **Measured, not implemented: `documents_fts.extracted_text` deduplication.** Against the real
  installation's corpus (`~/Library/Application Support/com.onrecord.studio/onrecord_studio_core.db`,
  measured 2026-09-29: 90 rows in `documents`, of which 84 have a non-null `extracted_text`), total
  extracted-text bytes equal unique extracted-text bytes exactly — zero duplicate documents.
  Deduplicating would have required rewriting 10+ direct-column read sites (full-text search,
  `SUBSTR`-offset chunk extraction, the orphan audit, purge) for a measured benefit of zero on this
  corpus. Not implemented; the same measurement should be redone if the corpus composition changes
  materially (e.g., the same agenda re-ingested across portals).
  Separately: `pragma_storage_info` shows this column stored `Uncompressed` in practice, contrary to
  an earlier assumption that DuckDB's default column compression already covered it (FSST does not
  currently apply here — an earlier answer claiming it already covered this column was wrong).
  Measured directly (same corpus and snapshot, same 84 rows, 365,086,281 raw UTF-8 bytes of
  concatenated `extracted_text`): standalone zstd over that text compresses to a 4.15:1 ratio (75.9%
  smaller) at level 3, and 6.38:1 (84.3% smaller) at level 19 — a real, substantial, and currently
  unclaimed lever, distinct from the zero-benefit dedup finding above. Not implemented, and two
  claims below this point are unverified, not established, and should not be treated as fact without
  checking DuckDB's storage-compression documentation for this exact `duckdb` crate version
  (`1.10505.0` in `Cargo.lock`) and this database's `storage_compatibility_version`: whether DuckDB's
  own automatic compression selection can already apply zstd to a column like this one (if so, the
  lever may be far cheaper than a hand-rolled column codec — the `Uncompressed` result above may
  reflect the analyzer's choice for this data, not an absence of the capability), and whether `lance`
  addresses this same gap (not evaluated in this session). `async-duckdb` does not address this gap
  regardless: it changes connection/query concurrency, not on-disk column compression.
- **Not measured: embedding chunk deduplication or quantization.** The installation's
  `onrecord_studio_vectors.db` was 12 KB at measurement time — no real embedding corpus exists yet
  to measure duplicate-chunk rate or f16/f32 top-k overlap against. Revisit once real usage
  populates it; implementing either change against no data would be a guess, not an engineering
  decision.
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

## 10. Citable evidence packets

`export_evidence_packet` (`commands/report.rs`, `report/packet.rs`) writes a Markdown file to read
and cite and a JSON file with the same content, into the managed report directory. It is built
from `report::builder::build_manifest`, the same pass the signed verifiable report uses, so the
basis label, the unassessed-versus-measured confidence state, the quote check against stored text,
PII masking, the 50-citation cap and the withholding of inference hypotheses, dismissed findings
and non-public excerpts are one implementation, not two.

What a claim carries: the basis in words (`ReportEvidenceBasis::label`, derived from
`FindingKind::is_legal_claim`, never chosen by the caller); the detector slug and the detector's own
explanation as the account of what was compared; the confidence as observed, where `Not assessed`
is never rendered as a number; and per source the file and extracted-text digests, the download URL,
the character range and the excerpt with its check status. Every field not recorded reads `not
recorded`. "What this does not show" is generated only from what was observed for that claim
(unchecked or withheld excerpt, missing digest or URL, no citation, no assessed confidence), plus
the fixed limit of its basis. The download URL is taken from `documents.download_url`, for public
documents only, with the query string and fragment removed because a signed query string can be a
bearer token. No per-document retrieval time is stored, so none is claimed.

**Gaps are never claims.** `PacketGap` shares no field with `PacketClaim` (no basis, severity or
confidence), renders under its own heading, and is not counted in the claim total. Gaps come from
`gaps::collect` without queueing acquisition, filtered to the detectors that produced a claim. A
detector whose sweep failed is named in the limitations, so an empty list is never the only signal.
An empty list is worded as "not a statement that none exist".

**Reproducibility.** `content_sha256` is the SHA-256 of the compact JSON of `PacketContent`:
schema version, portal scope, claims sorted by finding id, gaps sorted by key, withheld counts and
limitations. It excludes `generated_at`, every finding timestamp (`last_seen_at` moves on each
refresh), the author and the build version, so regenerating from unchanged findings gives the same
hash, and a changed quote, confidence, claim or gap gives a different one. Not assessed and measured
zero hash differently. A recipient recomputes it from the JSON (`report::packet::hash_matches`).
Limits: the packet is unsigned, so the hash shows change and not authorship; detectors are
identified by slug only because no detector version is recorded, and a different program build can
produce different content from the same findings (`build_version` is recorded but not hashed).
