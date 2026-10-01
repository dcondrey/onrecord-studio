# Research snapshot: the open file is the API

OnRecord Studio exports a read-only DuckDB file holding a stable projection of the evidence
corpus, with a signed manifest that names the exact corpus state it came from. Anything that
speaks DuckDB (Python, R, the `duckdb` CLI, Observable, Tableau) queries it directly: no server,
no rate limit, no credentials. The point is not convenience but citation: a figure published from
a snapshot carries a digest that lets anyone with the same snapshot re-run the query and get the
same rows.

Produced by `export_research_snapshot` (`src-tauri/src/commands/snapshot.rs`) from the projection
in `src-tauri/src/storage/snapshot_export.rs`. In the app: San Diego view, *Collection, coverage
and storage*, *Research snapshot for outside tools*.

## Files

```
~/Library/Application Support/com.onrecord.studio/reports/snapshots/
  onrecord-snapshot-20261001T031500Z.duckdb
  onrecord-snapshot-20261001T031500Z.duckdb.manifest.json
```

The `.duckdb` file is complete on its own. The manifest is an Ed25519-signed package in the same
format as every other signed export (`scripts/verify-evidence-package.mjs` verifies it), whose
`manifest` object holds:

| field | meaning |
|---|---|
| `schemaVersion` | `onrecord-research-snapshot-v1`; bumps only when a documented column changes meaning or disappears |
| `appVersion` | the OnRecord Studio version that wrote the file |
| `createdAt` | export instant, RFC 3339 UTC |
| `fileName`, `fileSha256` | the `.duckdb` file and its digest |
| `corpusStateDigest` | SHA-256 over one line per document, `portal_key\tid\textracted_text_sha256\n`, ordered by (portal_key, id) |
| `findingsDigest` | SHA-256 over one line per finding, `id\tlast_seen_at\t\n`, ordered by id, dismissed findings included |
| `tableRows` | row count per table at export |

Two snapshots with equal `corpusStateDigest` describe the same evidence corpus, whatever the app
version or export time. The signature proves the file is unchanged since export, not who exported
it; the key is the installation's self-managed export key.

The same values (except `fileSha256`, which cannot describe its own file) are inside the file as
`snapshot_manifest(key, value)`, with one `rows.<table>` entry per table.

## Tables

All timestamps are text as stored by the app (RFC 3339 or portal-native). `portal_key` is the
saved source's identifier and joins every table. Nullable columns are NULL when the app never
recorded the value; a NULL confidence means *not measured*, never zero.

| table | columns |
|---|---|
| `portals` | portal_key, display_name, base_url, source_kind, lifecycle_state, verified_at, last_synced_at |
| `agencies` | id, portal_key, name, department_name |
| `requests` | id, portal_key, rpid, title, status, agency_id, created_at, updated_at, closed_at, due_date, determination_at, determination_source |
| `statutory_extensions` | request_id, portal_key, notice_at, expected_determination_at, reason, notice_source, recorded_at |
| `documents` | id, request_id, portal_key, file_name, file_size, content_type, download_url, visibility, created_at, updated_at, raw_file_sha256, extracted_text_sha256, extraction_status, extraction_format, was_ocr_processed, shannon_entropy, entropy_sample_bytes, suspicious_data_density |
| `statutory_violations` | violation_id, portal_key, request_id, matched_statute_id, matched_version_id, mandate_id, classification, calculated_variance_metric, variance_days, confidence_basis_points, evidence_summary, supporting_document_ids_json, missing_evidence_json, classifier_version, review_status, first_detected_at, evaluated_at |
| `findings` | id, kind, severity, confidence_basis_points, portal_key, subject_agency_id, subject_request_id, subject_document_id, subject_entity_id, headline, explanation, citations_json, discovered_at, last_seen_at, dismissed_at, source_detector |
| `snapshot_manifest` | key, value |

Deliberately absent: extracted text, request descriptions, reviewer notes, local file paths, the
semantic index, the entity graph, imported analytical tables, drafts, tracked requests and case
files. Document identity travels as digests so a holder of the original can prove a match without
the snapshot carrying the content. Finding headlines, explanations and `citations_json` quotes are
included exactly as stored, so review them before sharing a snapshot outside the team.

## Querying

```sh
duckdb -readonly onrecord-snapshot-20261001T031500Z.duckdb \
  "SELECT a.name, count(*) AS violations
   FROM statutory_violations v JOIN requests r USING (request_id, portal_key)
   JOIN agencies a ON a.id = r.agency_id AND a.portal_key = r.portal_key
   WHERE v.classification = 'StatutoryDeadlineExceeded'
   GROUP BY 1 ORDER BY 2 DESC"
```

```python
import duckdb
con = duckdb.connect("onrecord-snapshot-20261001T031500Z.duckdb", read_only=True)
state = con.sql("SELECT value FROM snapshot_manifest WHERE key = 'corpus_state_digest'").fetchone()[0]
df = con.sql("SELECT kind, severity, count(*) n FROM findings WHERE dismissed_at IS NULL GROUP BY 1, 2").df()
```

Publish `state` beside any figure. To check a snapshot someone sent you, keep the two files
together and run the standalone verifier; it checks the signature and, when the `.duckdb` file is
beside its manifest, that the file hashes to the signed `fileSha256`:

```sh
node scripts/verify-evidence-package.mjs onrecord-snapshot-*.duckdb.manifest.json
```

## Guarantees and limits

- A snapshot is never overwritten; a second export in the same second fails.
- The file is written by the live connection through `ATTACH`, so it reflects the core at the
  instant of export and the app keeps running.
- Columns listed above keep their names and meanings within `onrecord-research-snapshot-v1`. New
  tables or columns may appear without a version change; removals or meaning changes bump it.
- A statistical column (`shannon_entropy`, performance measures) is a measurement; only
  `statutory_violations` rows and findings whose kind the app labels a legal claim are counted
  violations. The projection does not add interpretation the app itself would not show.
