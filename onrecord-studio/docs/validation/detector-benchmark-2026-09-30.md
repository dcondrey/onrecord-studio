# Autonomous detector benchmark on the San Diego corpus, September 30, 2026

Scope: run every registered detector against a copy of the real local database, grade every output against the cited source text, fix the false positives at their cause. Labels follow the research-claims rule: **measurement** (reproduced by the command below), **inference** (reasoned from measurements), **conjecture** (not tested).

## Result in one paragraph

Measurement: 21 detectors ran; 20 emitted nothing and one, `absence_of_records`, emitted 5 findings. All 5 were false (0 of 5 precise). Each was a detector error, not a document defect. After the fix the detector emits 0 findings on the same copy and the 5 findings are gone. No verified new finding surfaced. The honest reading is that this corpus does not exercise most detectors (see "Why zero is weak evidence"), so the benchmark measures false-positive behavior on public open-data and agenda documents, and says almost nothing about recall.

## Environment

| item | value |
| --- | --- |
| source | `~/Library/Application Support/com.onrecord.studio/onrecord_studio_core.db` (870,068,224 bytes, modified Sep 14) plus `.wal`, copied to a scratch directory and never opened in place |
| contents (measurement) | 91 documents with extracted text in 7 source portals, 123 `requests` rows all with status `published`, 0 `statutory_extensions`, 0 `statutory_violations`, 0 agencies. By extension: 60 pdf, 23 html, 4 csv, 2 json, 1 xlsx, 1 png |
| text volume (measurement) | 362 MB of the extracted text is one portal's four Get It Done CSV exports (largest 128 MB); everything else is about 3.6 MB |
| commit | 6333b3e plus uncommitted working-tree changes from parallel workstreams |
| toolchain | rustc 1.96.0, duckdb crate 1.10505.0 (bundled), macOS 27.0, Apple M4 |
| build profile | debug (`cargo test`). Runtimes below are debug-profile and were measured with up to three benchmark processes running concurrently; treat them as upper bounds, not throughput figures |

The corpus index (`onrecord_studio_corpus.db`) is not read by any detector in this suite; every detector reads the core database (`documents`, `documents_fts`, `requests`).

## Method and reproduction

The harness is `src-tauri/tests/detector_benchmark.rs`, an `#[ignore]`d integration test. It opens the database named by `ONRECORD_BENCH_DB`, runs the same migrations the app runs at launch (an older file lacks the findings tables), calls each detector's `run` directly, and never records findings, so the only writes are the migrations and DuckDB's own WAL checkpoint. Always point it at a copy.

```
S=<scratch dir>; mkdir -p $S
cp ~/Library/Application\ Support/com.onrecord.studio/onrecord_studio_core.db{,.wal} $S/
cd onrecord-studio
ONRECORD_BENCH_DB=$S/onrecord_studio_core.db \
ONRECORD_BENCH_OUT=$S/findings.json \
cargo test --manifest-path src-tauri/Cargo.toml --test detector_benchmark \
  -- --include-ignored --nocapture --test-threads=1
```

Optional: `ONRECORD_BENCH_ONLY=slug,slug` or `ONRECORD_BENCH_SKIP=slug,slug`. The companion test `registry_matches_refresh_all_findings` fails if a detector is added to `refresh_all_findings` without being added to the harness list. Output is a JSON array of `{detector, count, seconds, findings[]}`; per-detector lines are printed to stdout.

Two detectors are slow on this corpus, so a full run in a debug build takes about 40 minutes single-process (`contradictory_statement` 1,435 s, `inferred_redaction_identity` 892 s). Splitting the run with `ONRECORD_BENCH_SKIP` across copies of the database is what was done here.

Grading: for each finding, the cited document text was read from a read-only copy with the DuckDB CLI (`documents_fts.extracted_text`) around every token the finding relies on. TRUE requires the source text to support the finding as stated; FALSE means the source contradicts it or the detector's premise is wrong; UNVERIFIABLE means the source cannot decide.

## Per-detector table (measurement)

Graded n equals raw count everywhere; no sampling was needed. Precision is undefined where n is 0.

| detector | raw count | graded n | true | false | unverifiable | precision | runtime (s, debug) |
| --- | ---: | ---: | ---: | ---: | ---: | --- | ---: |
| held_back_response | 0 | 0 | 0 | 0 | 0 | n/a | 0.00 |
| boilerplate_denial | 0 | 0 | 0 | 0 | 0 | n/a | 5.3 |
| just_in_time_compliance | 0 | 0 | 0 | 0 | 0 | n/a | 0.00 |
| extension_abuse | 0 | 0 | 0 | 0 | 0 | n/a | 0.00 |
| contradictory_statement | 0 | 0 | 0 | 0 | 0 | n/a | 1435.1 |
| no_records_contradicted | 0 | 0 | 0 | 0 | 0 | n/a | 24.4 |
| **absence_of_records** | **5** | **5** | **0** | **5** | **0** | **0/5** | **61.9 (3.1 after fix, 0 findings)** |
| differential_treatment | 0 | 0 | 0 | 0 | 0 | n/a | 0.01 |
| metadata_vs_content | 0 | 0 | 0 | 0 | 0 | n/a | 0.01 |
| regenerated_response | 0 | 0 | 0 | 0 | 0 | n/a | 0.00 |
| email_thread_gap | 0 | 0 | 0 | 0 | 0 | n/a | 0.00 |
| evasion_paraphrased_denials | 0 | 0 | 0 | 0 | 0 | n/a | 4.6 |
| evasion_round_number_thresholds | 0 | 0 | 0 | 0 | 0 | n/a | 0.00 |
| evasion_redaction_at_limit | 0 | 0 | 0 | 0 | 0 | n/a | 0.01 |
| evasion_late_arriving_records | 0 | 0 | 0 | 0 | 0 | n/a | 6.3 |
| inferred_redaction_identity | 0 | 0 | 0 | 0 | 0 | n/a | 891.5 |
| cross_corpus_shared_language | 0 | 0 | 0 | 0 | 0 | n/a | 0.5 |
| cross_corpus_temporal_shift | 0 | 0 | 0 | 0 | 0 | n/a | 0.00 |
| cross_corpus_orphan_topic | 0 | 0 | 0 | 0 | 0 | n/a | 0.00 |
| cross_corpus_reused_denial_grounds | 0 | 0 | 0 | 0 | 0 | n/a | 0.00 |
| policy_violation | 0 | 0 | 0 | 0 | 0 | n/a | 1.4 |

`contradictory_statement` ran to completion once (1,435 s) in a separate full run; all other rows come from a second run of the same commit on a fresh copy. The two runs agreed on every detector they share.

## Graded findings: absence_of_records (5 of 5 FALSE)

All five findings group documents by `request_id` and treat any `item|invoice|resolution|ordinance N` token as a member of an agency numbering scheme.

| headline | cited documents | what the source text shows | grade |
| --- | --- | --- | --- |
| 21,447 ordinances missing between #1 and #21459 | 6223722556980981, 5374230528267211 (Get It Done CSV exports) | "#1" is "city ordinance 1-619-236-5500", a phone number inside a complaint cell. "#4" is "scooter ordinance 4 x 40 rule". "#21459" is a citizen's report citing a sidewalk-vending ordinance number. Three unrelated citizens' free text, not a numbering scheme. | FALSE |
| 205,684 resolutions missing between #1 and #205688 | same CSVs | "resolution.1." is the end of "no resolution." followed by a list marker; "resolution. 6" is "no resolution. 6 months ago". The endpoint "#205688" was not located with a bounded-context search, so it is unread; the grade rests on the members that were read, which are prose. | FALSE |
| 403,230 invoices missing between #6 and #403238 | 6223722556980981 | "#403238" is a tow invoice number a complainant typed in one cell; "#7" is "invoice 7/26/21", a date. | FALSE |
| 523 items missing between #1 and #530 | same CSVs | "item 5$", "item 2 weeks ago", "ITEM 1." in complaint prose. The endpoint "#530" was not located with a bounded-context search and is unread; the grade rests on the members that were read. | FALSE |
| 298 items missing between #10 and #331 | 5815056345952905 (`9-15-26_2.pdf`, the September 15, 2026 Council agenda) | The agenda numbers items in category blocks: Item 10 (minutes), 30 and 31 (proclamations), 50 to 56 (ordinances), 100 to 106 (resolutions), 200 and 201, 330 and 331 (hearings). About 25 numbers exist by design across a span of 322. The unused numbers are not withheld records, and an agenda is not a release. | FALSE |

Root causes, both in `detectors/absence_of_records.rs`: (1) structured-data exports were scanned as prose; (2) the module documentation promises a "tight" sequence, but any span with at least 3 members produced a finding, however sparse. Measurement: the four CSV findings cite only the two Get It Done exports; the agenda finding has 25 members in a span of 322, a missing fraction of 92%.

### Fix and regression evidence

- Documents whose file name ends in `.csv .tsv .json .ndjson .xls .xlsx` are skipped.
- A sequence whose missing fraction exceeds 0.5 is not emitted. The existing tests (3 members in a span of 4, 10 in 12) still pass; the Serious threshold of 0.2 is unchanged.
- Test `open_data_rows_and_block_numbered_agendas_are_not_sequences`. Proven by removing each guard separately: with only the tabular guard disabled it fails with `1 invoice missing between #1 and #4`; with both disabled it fails with `320 items missing between #2 and #331`. With the fix all 9 tests in the module pass.
- Re-run on a fresh copy: 0 findings in 3.1 s.

## Why zero is weak evidence (inference)

- Every one of the 123 `requests` rows is a wrapper for a public-data source with status `published`. There are no agencies, no closure dates, no statutory extensions, and no response documents. Detectors that need a request lifecycle (`held_back_response`, `just_in_time_compliance`, `extension_abuse`, `differential_treatment`, `evasion_late_arriving_records`, `regenerated_response`, `email_thread_gap`) have no input. Zero from them is what they should return, and it would be the same if they were broken.
- There is no positive control in this corpus. Each detector's recall is exercised only by its synthetic unit tests, not by real data. This benchmark does not measure recall for any detector.
- Conjecture: `boilerplate_denial`, `evasion_paraphrased_denials`, and `cross_corpus_*` would need a corpus of real agency denial letters to produce output.

## Performance observations

- Measurement: `contradictory_statement` (1,435 s) and `inferred_redaction_identity` (892 s) each ran for many minutes because the 362 MB of CSV text is inside their bounded sweeps; the bounds cap document counts (`MAX_DOCUMENTS`, `MAX_CANDIDATE_DOCS`), not bytes. `refresh_all_findings` is documented as bounded and responsive; on a corpus with large tabular files it is not. Measured in a debug build, so a release build will be faster by an unmeasured factor.
- Inference: skipping tabular exports in those two detectors, as done for `absence_of_records`, would remove almost all of that time and cannot remove a true legal-record signal, since row content is not agency prose. `contradictory_statement.rs` is not editable by this workstream; `inferred_redaction_identity.rs` was left unchanged because it produced no false positive.

## Findings a human had not caught

None. The only output was five false positives, now fixed. No document ID is listed because no detector produced a finding whose cited text supports it. This benchmark did not add to the 2026-09-26 write-up.

## What remains

- Positive controls: run the suite on a corpus that has agency responses (a real records-request release set, or a labelled synthetic set) so recall can be measured and the request-lifecycle detectors can be graded at all.
- Add a byte budget, or skip tabular exports, in `contradictory_statement` and `inferred_redaction_identity` (the first is owned elsewhere).
- Release-profile timing of the full suite.
