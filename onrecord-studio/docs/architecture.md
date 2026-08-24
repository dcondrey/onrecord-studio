# OnRecord Studio - Deep Architectural Blueprint & Engineering Specification

This document provides an exhaustive, production-grade breakdown of the multi-tiered, asynchronous, resource-isolated desktop runtime environment engineered for `onrecord-studio`.

---

## 1. Global System Topology

OnRecord Studio utilizes a strict decoupled structural framework powered by Tauri v2. Performance-heavy operations—such as raw network serialization, disk persistence, column-oriented analytical calculations, and multi-format document text extraction—are delegated entirely to a multi-threaded, memory-safe native Rust backend. The frontend acts exclusively as an accelerated, zero-state visualization layer.

```text
+-----------------------------------------------------------------------------------+

|               UX VISUAL LAYERS (React / TypeScript / Tailwind CSS)                |
|  - Reactive State Manager (Zustand Atomic Nodes)                                  |
|  - High-Velocity Visualizations (Apache ECharts via WebGL/Canvas Cores)           |
|  - Real-Time System Log Terminal Render Engines                                   |
+-----------------------------------------------------------------------------------+
                                         |
                                         | Tauri v2 Inter-Process Communication (IPC)
                                         | Secure Serialized JSON Command Buffers
                                         v
+-----------------------------------------------------------------------------------+

|               NATIVE SYSTEM HARDWARE BACKEND ENGINE (Rust Core)                   |
|  - Multi-Threaded Task Coordinator & Pipeline Scheduler (Tokio Runtime)           |
|  - Subdomain Isolation & Exponential Backoff Controllers (Reqwest Connection Pool)|
|  - Memory-Safe Compound Binary Decompressors & Document Tokenizers                |
+-----------------------------------------------------------------------------------+
                   /                                             \
                  v                                               v
+---------------------------------------+       +---------------------------------------+

|    SQLITE CORE RELATIONAL DATABASE    |       |   DUCKDB OLAP DATA ENGINE WORKSPACE   |
| - Mode: Write-Ahead Logging (WAL)     |       | - Type: Vectorized Column-Oriented    |
| - Pragma Cache Allocation: ~64MB      |       | - Context: Direct SQLite File Attachment|
| - Virtual Index Tables: FTS5 Porter   |       | - Math: High-Speed Array Aggregations  |
+---------------------------------------+       +---------------------------------------+
```

---

## 2. Low-Level Ingestion Core & Network Isolation

To safely query government servers without trigger blocks or resource constraints, the ingestion system applies specific traffic patterns:

### A. Thread Queue Coordination (`tokio::sync::mpsc`)
The system routes data tasks via an asynchronous bounded Multi-Producer Single-Consumer channel capped at 10,000 active nodes. This prevents uncontrolled memory growth when an investigator schedules heavy multi-year municipal sweeps.

### B. Subdomain-Aware Token Bucket Rate Limiter
The system manages traffic thresholds individually per municipal subdomain (`sandiego` vs. `sandiegohousingcommission`) to isolate network footprints:
* **Capacity Bounds:** Each bucket holds a maximum of 5 burst tokens.
* **Refill Cadence:** Tokens replenish linearly at exactly 1.0 token/second.
* **Thread Throttling:** When a bucket is exhausted, worker execution threads drop into a non-blocking `tokio::time::sleep` state for 200ms before re-evaluating the bucket capacity.

### C. Connection Lifecycle Pooling (`reqwest`)
The native client pools connections to minimize TCP handshake overhead:
* **Idle Window:** Idle connections are held for a maximum of 90 seconds.
* **Host Allocation Constraints:** Outbound traffic is capped at exactly 5 connections per endpoint.
* **Header Signatures:** Connections override default configurations to mirror standard macOS Safari traffic parameters:
  ```text
  User-Agent: Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.0 Safari/605.1.15
  Accept: application/json, text/plain, */*
  ```

---

## 3. High-Velocity Storage & Search Tier Execution

### A. Relational Data Management (SQLite WAL Engine)
To maintain interface responsiveness during heavy indexing cycles, SQLite is configured with optimized execution pragmas:
```sql
PRAGMA foreign_keys = ON;
PRAGMA journal_mode = WAL;          -- Permits simultaneous read operations during backend writes
PRAGMA synchronous = NORMAL;       -- Balances disk synchronization intervals with bulk append speeds
PRAGMA cache_size = -64000;        -- Allocates a persistent 64MB memory cache pool
```

### B. Sub-Millisecond Search Layer Indexes (FTS5 Unicode Integration)
Full-text search queries execute against a virtual `documents_fts` schema table configured with a Porter stemming tokenizer:
* **Stemming Engine:** Automatically reduces search tokens to their word roots (e.g., querying `"sweeping"` matches instances containing `"sweep"` or `"swept"`).
* **Automated Data Sync Triggers:** To keep indices updated without application overhead, database triggers update the full-text search table immediately when text extraction succeeds on a document record:
  ```sql
  CREATE TRIGGER trg_documents_fts_insert AFTER UPDATE OF local_path ON documents
  WHEN NEW.local_path IS NOT NULL AND OLD.local_path IS NULL
  BEGIN
      INSERT INTO documents_fts(document_id, portal_subdomain, file_name, extracted_text)
      VALUES (NEW.id, NEW.portal_subdomain, NEW.file_name, 'Extraction pending backend stream...');
  END;
  ```

---

## 4. Multi-Format Structural Document Parsing Architecture

When downloadable assets drop into the processing pipeline, the system evaluates their file structures asynchronously using targeted extraction drivers:

1. **PDF Layer Extractor (`lopdf`):** Extracts raw text tokens from clean documents page-by-page, reassembling text layout strings into uniform character structures.
2. **Spreadsheet Grid Normalizer (`calamine`):** Parses raw binary `.xlsx` and multi-sheet grids. Iterates cell-by-cell, flattens numerical formulas or float metrics, and normalizes rows into clean, indexable tables.
3. **Outlook Email Binaries Parser (`msg-parser`):** Decouples Outlook compound structures. Extracts headers (`From`, `To`, `Cc`, `Date`), strips layout HTML blocks from the message body, and routes embedded inner attachments back to the core parser grid.

---

## 5. Security Permissions Sandbox (`capabilities/main.json`)

Tauri v2 enforces explicit runtime capability profiles to maintain a tight security sandbox:
* **`core:path:default` / `core:event:default`:** Limits application execution access to standard system paths and window event pipelines.
* **IPC Isolation Scopes:** Restricts frontend interaction rules to exactly three secure command routes (`allow-start-subdomain-sync`, `allow-query-audit-records`, `allow-export-case-dataset`), preventing unauthorized command injection vulnerabilities inside the native web-view.
