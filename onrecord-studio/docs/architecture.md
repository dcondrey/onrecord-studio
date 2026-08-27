# OnRecord Studio - Architecture

Every number in this document is a named constant in the source. The citation after each one is
authoritative; if the two disagree, this document is wrong.

---

## 1. Topology

A Tauri v2 desktop application. The React frontend renders and dispatches; all I/O, parsing,
storage, and analysis happen in the Rust backend.

```text
+---------------------------------------------------------------+
|  React / TypeScript / Tailwind                                |
|  - Zustand store (src/store/useAppStore.ts)                   |
|  - Hand-written SVG charts (src/components/charts/)           |
|  - Every invoke() call routed through src/ipc.ts              |
+---------------------------------------------------------------+
                            |
                            |  Tauri v2 IPC: 27 commands, typed on both sides.
                            |  scripts/check-ipc-contract.mjs fails the build if the
                            |  Rust handler list and NativeCommandMap disagree, or if
                            |  any invoke() appears outside src/ipc.ts.
                            v
+---------------------------------------------------------------+
|  Rust backend                                                  |
|  - Tokio multi-threaded runtime; blocking work via            |
|    blocking::spawn_with                                        |
|  - Per-subdomain token-bucket limiter (ingestion/limiter.rs)   |
|  - Format-dispatched parsers (parsers/)                        |
+---------------------------------------------------------------+
                            |
                            v
+---------------------------------------------------------------+
|  DuckDB core (onrecord_studio_core.db)                         |
|  - The single persistent store. Not SQLite.                    |
|  - One writer connection, serialized behind a tokio Mutex      |
|  - Analytics read through Connection::try_clone siblings       |
|  Corpus index (onrecord_studio_corpus.db), a rebuildable       |
|  projection of the core.                                       |
+---------------------------------------------------------------+
```

SQLite appears in exactly two places, neither of them the application's own storage:
`parsers/database.rs`, which opens investigator-supplied SQLite files read-only under
`PRAGMA query_only = ON` and `PRAGMA trusted_schema = OFF`; and `storage/legacy_import.rs`, the
one-way upgrade path described in section 3.

---

## 2. Ingestion

### A. Queue
Portal work is dispatched over a bounded `tokio::sync::mpsc` channel.
`PORTAL_QUEUE_CAPACITY = 2_048` (`ingestion/queue.rs`) bounds memory when an investigator schedules
a large multi-year sweep. Durable work is held in the `ingestion_task_frontier` table so an
interrupted run is recovered into a new run at startup rather than lost.

### B. Per-subdomain rate limiting
One token bucket per portal subdomain, so traffic to separate agency, municipal, nonprofit, or
publisher hosts is throttled independently (`ingestion/limiter.rs`). San Diego hostnames are only
fixtures for the initial use case; the ingestion model is region-neutral.

Burst capacity and refill rate are **per-policy, not fixed**. Validation bounds them:
`burst_capacity` must be in `1..=100`; `refill_tokens_per_second` must be in
`MINIMUM_REFILL_TOKENS_PER_SECOND = 0.001 ..= MAXIMUM_REFILL_TOKENS_PER_SECOND = 100.0`. A waiting
worker sleeps for the computed time to the next token, capped at
`MAXIMUM_ACQUIRE_SLEEP = 60s`. Raising a policy's burst capacity refills under the old policy first
and then clamps, so a policy change cannot mint tokens retroactively.

### C. HTTP client
`ingestion/client.rs`, reqwest with rustls (no OpenSSL, no platform TLS), gzip/brotli/deflate/zstd
response compression.

* `connect_timeout_seconds = 10`, `request_timeout_seconds = 45` (both configurable, both must be
  non-zero).
* `pool_idle_timeout` 90s, `pool_max_idle_per_host` 8.
* `redirect::Policy::none()`. Redirects are **not** followed automatically: each hop is returned to
  the caller and re-validated, so a redirect cannot bypass the URL safety check.
* User-Agent is `OnRecord-Studio/1.0 public-records-audit-client`. The client identifies itself
  honestly and does not impersonate a browser.

### D. Crawl policy
`CrawlPolicy` (`types/source.rs`) defaults: `maximum_pages = 5_000`, `maximum_depth = 8`,
`maximum_resources = 100_000`, `follow_supported_external_platforms = true`.

---

## 3. Storage

### A. DuckDB core
Opened once by `DuckDbRepository::new` (`storage/sqlite.rs` — the file name is historical; the
implementation is entirely DuckDB). Configuration is three `SET` statements, not SQLite pragmas:

```sql
SET preserve_insertion_order = false;
SET threads = 1;
SET enable_progress_bar = false;
```

`threads = 1` is instance-wide, so analytical queries are single-threaded too. That is a deliberate
desktop-responsiveness trade-off and a known limit on large-corpus query latency.

Writes are serialized through one connection behind a `tokio::sync::Mutex`. Analytics nodes take
additional connections to the **same instance** via `DuckDbRepository::analytics_connection`
(`Connection::try_clone`). This is load-bearing: a connection that instead `ATTACH`es the core file
observes a snapshot frozen at attach time and never sees a later write. Pinned by
`analytics_nodes_observe_writes_made_after_they_were_bound`.

### B. Upgrading a pre-DuckDB installation
Installations from before the storage migration hold a SQLite file at the core path, which DuckDB
cannot open. `storage/legacy_import.rs` classifies the file by header bytes before anything opens
it, and converts a legacy core in place:

1. Build the DuckDB schema in a sibling `*.migrating` file.
2. `ATTACH` the legacy file `(TYPE SQLITE, READ_ONLY)` — the original is never written to.
3. Copy shared tables in foreign-key dependency order, inside one transaction.
4. Verify per-table row counts **before** commit.
5. Retain the original as `*.legacy-sqlite.bak`, then move the new core into place.

Any failure leaves the original exactly where it was and removes the partial core, so the operation
is safe to retry. A file matching neither engine's header is never overwritten; startup refuses.

### C. Search
`documents_fts` is a **plain DuckDB table**, not an FTS5 virtual table, maintained transactionally
by the repository (`migrations/0001_initial_schema.sql`). The live query path is a case-folded
substring scan (`storage/sqlite.rs`): no stemming, no ranking, no text index.

TODO(search): This is a full table scan and will not hold at corpus scale. Add a versioned DuckDB
search index before relying on engine-provided ranking or tokenization.

---

## 4. Parsers

`parsers/format.rs` detects the container and routes to a bounded extractor: `lopdf` for PDF,
`calamine` for spreadsheets, `msg_parser` for Outlook `.msg`, plus archive, RTF, HTML, Office
package, plain-text, and salvage paths. Nested containers route back through the same dispatcher
under depth and size bounds.

All parser input is untrusted: documents arrive from arbitrary government portals and
investigator-supplied URLs.

---

## 5. Security model

**IPC.** Tauri v2 capabilities gate `core:` and plugin commands. They do **not** gate
`#[tauri::command]` handlers registered through `generate_handler!`, so all 27 application commands
are reachable from the webview by design. `capabilities/main.json` grants `core:path:default`,
`core:app:allow-set-app-theme`, `core:event:default`, `core:window:default`,
`core:webview:default`, and `dialog:allow-open`.

The consequence: **each command body is the trust boundary.** Validation belongs there, not in the
manifest. `scripts/check-ipc-contract.mjs` enforces that the two sides agree and that no `invoke()`
escapes `src/ipc.ts` — a typing and review discipline, not a sandbox.

**CSP** (`tauri.conf.json`):

```
default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline';
img-src 'self' data:; connect-src 'self' ipc: http://ipc.localhost
```

No `unsafe-eval`, no `unsafe-inline` on scripts, no wildcards. `style-src 'unsafe-inline'` is a
Tailwind concession.

**Defaults relied upon:** `withGlobalTauri` is absent and therefore false, so the webview gets no
`window.__TAURI__`. `assetProtocol` is absent and therefore disabled.

**Known gaps.** `ingest_local_files` (`commands/ingest.rs`) accepts arbitrary filesystem paths from
the frontend, so the `dialog:allow-open` mediation is advisory rather than enforced.
`macOSPrivateApi: true` is required by the transparent window and rules out Mac App Store
distribution.
