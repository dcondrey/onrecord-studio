# Local vector search and SQL analysis

Status: implemented September 13, 2026.

## Embeddings

OnRecord Studio uses a rebuildable local vector projection in `onrecord_studio_vectors.db`.
Original documents and extracted text remain in the core evidence store; deleting or rebuilding the
vector database does not delete evidence.

Pinned configuration:

- Runtime: `fastembed` 6.1.0 with ONNX Runtime, Rust only.
- Model: `Qdrant/bge-small-en-v1.5-onnx-Q`.
- Upstream source model: `BAAI/bge-small-en-v1.5`.
- Dimension: 384.
- Metric: cosine over L2-normalized vectors.
- Chunking: `onrecord-semantic-chunks-v1`, 1,600 characters with 160-character overlap.
- Stored compatibility fields: model id, model revision, dimension, metric, normalization,
  chunking version, source content hash, raw-file hash, chunk hash, and character offsets.

Model installation is explicit from the Analysis workspace. The first initialization downloads the
model into the app data directory under `models/embeddings`; later launches use that cache offline.
The app records an artifact manifest hash over the installed files and reports missing or failed
initialization in the UI. If the cache is interrupted or corrupted, initialize the model again; if
that still fails, remove only the model cache directory and reinstall. Do not remove the core
database to repair model assets.

Indexing is bounded, cancellable, and incremental. A document is skipped when its extracted-text
hash already has vectors for the same model and chunking version. Rebuild clears only the vector
projection. Model upgrades intentionally do not mix vectors: a new model revision writes a separate
projection.

Current search uses exact vector scan. This is deliberate for the measured starter corpus size in
the research notes: it avoids a second vector engine and keeps startup/storage recovery simple. If
the corpus grows enough for exact scan to dominate latency, add an ANN projection as another
rebuildable index, not as the evidence store.

## Hybrid retrieval

Hybrid search combines semantic results with existing lexical request/document search. Scores are
not added directly. Semantic results are ranked by cosine similarity; lexical fallbacks use
reciprocal rank. Result IDs are stable, and vector passages keep document id plus character
start/end offsets. Page coordinates are not invented when extraction did not provide them.

Filters currently supported through IPC: portal/source, agency name, jurisdiction, document type,
and date range. Jurisdiction labels intentionally distinguish City of San Diego, SDPD, San Diego
County Sheriff, university police, and neighboring municipalities. If semantic search cannot run,
the UI keeps lexical search usable and labels those results as lexical fallback/exact matches.

## Analytical datasets

CSV imports create two things:

- A durable snapshot ledger in the core DuckDB database: source path/URI, retrieval time, SHA-256,
  observation period, partial-dataset flag, validation summary, original row payloads, normalized
  row payloads, and column mappings.
- An approved read-only SQL table named `analysis_<family>_<hash>`, plus an
  `analysis_<family>_latest` view for templates.

Initial dataset families:

- RIPA stops and companion search-basis tables.
- SDPD calls for service and codebooks.
- SDPD NIBRS offenses.
- City operating budget and budget dimension tables.

The importer preserves original column names in `analytical_columns` and uses normalized aliases
in query tables. It validates dates, timestamps, identifiers, categories, and monetary amounts
where the schema mapping identifies them. Malformed rows are recorded in `analytical_rows`; they
are not silently coerced into defaults.

Domain guardrails encoded in templates and validation notes:

- RIPA keys include collection context and the within-stop person id; PID is not a global person.
- RIPA companion joins can multiply rows, so templates state the denominator.
- Calls, offenses, incidents, cases, and stopped-person rows are separate counting units.
- Public calls-for-service rows exclude publisher-documented categories.
- Budget fiscal year and budget cycle remain separate. Budget allocations are not actual spending.

## Restricted SQL

User SQL is parsed with `sqlparser` using the DuckDB dialect. Only one `SELECT` query is accepted.
The parser rejects mutation statements, arbitrary tables, table functions such as file readers,
attachments, extension loading, and non-approved sources. The execution connection disables DuckDB
external access before preparing the query. Exports are separate controlled application actions.

Approved tables are `analysis_*`, `analytical_snapshots`, and `analytical_columns`. Row output is
bounded by the command limit, and CSV export re-executes through the same restricted path.

Numerical answers in the application should come from executed SQL. Embeddings can retrieve
definitions and context; they are not authoritative for counts.

## Licenses and sources

- `fastembed` crate: Apache-2.0, per crates.io metadata checked September 13, 2026.
- `Qdrant/bge-small-en-v1.5-onnx-Q`: Hugging Face model card reports Apache-2.0.
- `BAAI/bge-small-en-v1.5`: Hugging Face model card reports MIT.
- San Diego Open Data source families in the research catalog linked ODC-PDDL at the time of the
  September 13, 2026 verification. Individual City PDFs and third-party attachments still need
  file-level rights review.

## Evaluation

No retrieval scores are claimed in this implementation. Deterministic tests cover SQL restriction,
RIPA within-stop PID key behavior, budget-cycle template handling, Unicode vector chunk offsets,
and vector dimension persistence checks. Real-model inference is available through the application
initialization path and should be exercised as an opt-in integration check because it downloads
model artifacts.

Recommended next evaluation file: a small reviewed San Diego set with evidence spans,
paraphrases, exact identifiers, wrong-jurisdiction examples, outdated-policy examples, and
unanswerable questions. Compare lexical-only and hybrid retrieval on the same frozen corpus and
report actual recall, citation correctness, latency, and memory.

