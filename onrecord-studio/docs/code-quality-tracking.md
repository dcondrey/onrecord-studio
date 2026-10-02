# Code quality improvement tracking

Started from a 28-component, 10-metric code audit (2026-10-01), scored strictly from reading
actual code, never from this or other docs. Test coverage is intentionally excluded from the
working metric set below — raising it is not a goal of this effort; only add a regression test
when fixing a real logic/security/concurrency defect.

Metrics in scope: Correctness, Error Handling, Security/Input Validation, Concurrency/Async-Identity
Safety, Performance/Efficiency, Architecture/Invariant Adherence, API/Contract Integrity,
Maintainability/Complexity, Comment/Code Clarity.

## Already fixed (commits on main, chronological — check `git log` for the authoritative state)

- `3a36489` residualAuditGate regression test (DocumentViewer.tsx)
- `b6e3af8` Analysis & Findings UI: hybrid-search request gate, EntityGraph alias-combine gate,
  PatternExplorer kind-filtered count bug, EntityGraph "not connected" overclaim wording
- `bcad79e` Tracker UI: gated `mutate()`, missing `sourceUrl` maxLength, surfaced portal-list error
- `c6806f0` legacy_import.rs: removed a runtime extension-download fallback, fixed a swallowed
  root-cause error, corrected a doc comment
- `d4e34ac` Research module: `label_in_quote` substring-match bug, a no-op test given a real
  assertion, an infinite-retry bug on persistent gap-collection failure, a negative-offset schema
  gap, a doc comment overclaiming where meeting dates come from
- `3dd974b` PDF recovery: bound-checked a declared xref offset before slicing (was a reachable panic)
- `174678e` claim_validation.rs: the most consequential defect found — a `difference == 0` sentinel
  let several claim kinds report "Consistent" for a claim that was actually false; also fixed an
  "over"/"year over year" phrase collision
- `c543652` financial.rs: surfaced a silently dropped dropdown-population query failure
- `811f377` provenance.rs (redacted a leaked SDK error), licensing/direct.rs (closed a token-cache
  race), resolution/ledger.rs (added a cross-process flock, mirroring integrity.rs's pattern),
  storage/monitoring.rs (cut a per-row allocation)
- `33b9c47` casework.rs: batched N+1 per-citation queries; maintenance.rs: stopped rehashing every
  compaction candidate just to rank by file size
- `894332d` analysis/detectors: extracted shared `split_units`/`jaccard`/`load_agencies` helpers
  (new `text_shared.rs`); left the two detectors' actual shingle-hash algorithms separate since
  they're genuinely different, not duplicated
- `fee5b3e` compression_oracle.rs: a truncated ZIP member read is now flagged `coverage_complete =
  false` instead of silently overclaiming a complete scan
- `46ab5ca` LicenseProvider.tsx: deduplicated 5x-repeated async-gate boilerplate into one helper
- `e3472a2` reformatted three dense, undiffable single-line JSX blocks (CaseworkPanel,
  LegalAuthorityManager, LegalCompliancePanel) — formatting only, no logic change

- `17d509f` DocumentAnnotations.tsx: split one shared gate across 4 unrelated actions
  (saveReviewer/remove/changeStatus/copy) into 4 independent gates
- `ef25790` ResearchWorkspace.tsx: reformatted dense single-line JSX blocks (formatting only)
- `7b05600` useGatedLoad.ts: held answer is keyed to its key, so the render after a key change no
  longer shows the previous key's data (affected layers, useCaseScope, AttentionFeed, CaseSwitcher)
- `26ca930` useObjectSearch.ts: a failed findings/entities snapshot read is retried, not cached
  for the palette session as an empty corpus
- `5af0487` LegalAuthorityManager.tsx: version comparison shows a held detail only when it is the
  selected version, so a selector change no longer displays the previous version's text
- `68e81fd` research/model.rs: daily-call slot reserved after every local-only failure; still
  before dispatch_current's recheck (an await there could send to a just-deselected provider), so a
  policy change during the reservation await still costs a slot
- `7173585` searchResults.ts: copied search record labels its corpus digest as read at copy time
  (per-search reads would add an unmeasured full-corpus scan and still race; not every corpus
  write bumps `evidenceRefreshRequests`, so it can't gate this)
- `149bdf0` sanDiegoWorkflow.test.tsx: valid meeting-calendar mock (`[]` crashed AttentionFeed)
- `9d3ff4d` storage/review.rs: relationship evidence-check errors propagate (were shown as "source
  needs review"); each source report decompressed once, not once per relationship
- `cc6c6fb` commands/account.rs: seat email validation rejects `a@@b`, `@a@b`, `a@b@`, whitespace
- `7f6ed0a` commands/semantic.rs: hybrid_search lexical dedup via HashSet (was quadratic over up
  to 5,000 rows per palette query; not benchmarked)

Two `sanDiegoWorkflow.test.tsx` full-`<App />` tests time out under full-suite CPU load (pre-existing,
reproduced with pre-`7b05600` code). Run the gate's vitest step as `npx vitest run --maxWorkers=2`;
with it all 597 tests pass (2026-10-02, `55fa447`).

## Deliberately deferred — do not attempt without explicit sign-off from the user

Large single-file structural splits in the same risk class as the former `storage/sqlite.rs`
(13k lines) / `ingestion/queue.rs` (8.4k lines), which were split by a peer session during this
effort:

- `ingestion/client.rs` (4441 lines, many connector decoders mixed in one file)
- `parsers/unredactor.rs` (3130 lines)
- `storage/financial.rs`'s `analyze()` function (~550 lines, one function)
- `components/dashboard/EntityGraph.tsx` (789 lines, several concerns)

If asked to improve these, use the `split-module` skill and confirm scope with the user first —
they carry real regression risk and are a different class of work than a scoped bug fix.

## Known-justified — not defects, don't re-flag

- `provenance.rs`: the sidecar-manifest fallback gap and the always-empty trust-anchor list are
  documented, intentional limitations (capping verification at `Valid`, never `Trusted`), not
  fixable without a larger provenance feature.
- `hooks/useSourceConnector.ts` bundling six flows into one 558-line hook: justified by its own
  doc comment (shared state across the flows).
- `analysis/detectors/boilerplate_denial.rs` vs `evasion_paraphrased_denials.rs`: different
  shingle/hash constructions by design (FNV-1a vs SipHash, different token filtering) — only the
  shared `jaccard()` comparison was safe to unify.
- `scripts/check-ipc-contract.mjs` only checks command-name parity, not argument/type shape — by
  design; type-shape fidelity is `scripts/generate-contracts.mjs --check`'s job.
- `scripts/sourceGuidance.ts`'s `parsePatternList` already compiles and rejects invalid
  regex/glob patterns client-side before any IPC call — a prior audit note questioning this was a
  false concern.

## Shared-tree etiquette — this repo has other Claude sessions working in it concurrently

1. Run `ListAgents` at the start of a session and whenever starting a new file area.
2. `git status` before editing — if a file is already modified and you don't recognize the change
   as your own prior work, message the owning session (if known) before touching it. Never discard
   or stage another session's uncommitted changes.
3. File ownership claims as of 2026-10-02 (confirm current state — these expire as sessions finish):
   `onrecord-studio-78` claimed `src/store/useAppStore.ts`,
   `src/components/dashboard/{Sidebar,Header,HeaderControls,CommandPalette,OverviewDashboard,
   StoryDesk,FindingsInbox,ActivityDashboard,MainLayout}.tsx`, `src/commands/registry.ts`,
   `src/components/objects/*`, `src/hooks/useNavigationHistory.ts`.
4. Never `git add -A`. Commit by explicit path, in small coherent groups, with a message describing
   the defect and the fix.
5. The cargo build uses one shared target directory across all sessions — expect queued, slow
   compiles when others are building concurrently. That's contention, not a failure; don't retry
   blindly, just wait for the lock.

## Process for each iteration

1. Pick one component/file not already covered above by reading its **current** code — never rely
   on this file or `docs/*.md` for correctness claims, only for what's already been done.
2. Find one concrete, evidence-backed defect: correctness, error handling, security, concurrency,
   performance, architecture/invariant adherence, API contract, or maintainability/clarity. Not a
   vague "could be nicer."
3. Make the smallest correct fix. No speculative refactors, no unrequested scope, no new
   dependencies.
4. Add a regression test only if the fix changes real logic/security/concurrency behavior.
5. Run the narrowest relevant check per fix (e.g. `cargo test --manifest-path src-tauri/Cargo.toml
   --lib <module>::`, or `npx vitest run <file>`), not the full gate.
6. Commit immediately with explicit paths.
7. Append one line to "Already fixed" above.
8. Periodically (not after every single fix), run the full gate from
   `/Volumes/A/onrecord-studio/onrecord-studio`:
   ```
   cargo fmt --manifest-path src-tauri/Cargo.toml --check
   cargo clippy --manifest-path src-tauri/Cargo.toml --lib --all-targets
   cargo test --manifest-path src-tauri/Cargo.toml --lib
   npx tsc --noEmit
   npx eslint src --max-warnings 0
   npx vitest run
   node scripts/check-ipc-contract.mjs
   node scripts/check-help-coverage.mjs
   ```
