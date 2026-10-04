# Competitive positioning: the superior form of each planned capability

Companion to `investigative-workbench-architecture.md`. That document decides how the workbench
is built; this one decides what each outward-facing capability should be, measured against the
tool that currently owns the category (MuckRock, Aleph, DocumentCloud, Everlaw, MAXQDA, Amped,
Pinpoint). The recurring move is the same in every section: where a competitor offers a
convenience, OnRecord offers the same capability as evidence, with the claim's basis recorded
and reproducible.

Every "already exists" statement below was verified against the working tree on 2026-08-30,
with file:line evidence; corrections from that audit are integrated in place rather than
footnoted. Re-verified against the tree on 2026-09-30 after two weeks of active development;
drifted line citations were corrected and the findings below updated in place. Paths are
relative to `onrecord-studio/`.

## 1. Filing: the request becomes evidence

MuckRock's record that you filed is a row in their database. OnRecord should file through the
operator's own channels (their email account, or portal submission forms) and capture the
transmission itself as a signed evidence package: what was sent, the instant, the SMTP or
portal confirmation, response headers. The statutory clock then starts from a provable filing
instant instead of a claimed one. Follow-ups become the killer feature: MuckRock's are dumb
timers, but the statutory engine can generate follow-ups that are legal instruments, citing the
exact statute, the elapsed-day count, and the invoked extensions, with the violation math
attached.

Ground truth. The letter's numbers all exist:
`evaluate_statutory_compliance_with_determination_at`
(`src-tauri/src/analysis/timeline.rs:101-228`) returns elapsed and allowed calendar days,
extension status and reason, violation markers, the statute citation, and a rendered evidence
summary, and `export_legal_brief` (`src-tauri/src/commands/export.rs:160-333`) already consumes
all of it, signs the package (`export.rs:273`), and emits it as an Ed25519-signed
`.evidence.json` file (`export.rs:283-296`). So the follow-up generator is largely a new template
over existing outputs. Three limits bound the rest of the item. The engine is single-statute: only
the CPRA 10-day determination clock is modeled (`src-tauri/src/analysis/statute.rs:40-47`). The
NextRequest machinery (`src-tauri/src/ingestion/client.rs:396-558`, `876-944`,
`src-tauri/src/types/nextrequest.rs`) is read-only for filing purposes. One correction: `client.rs`
is not POST-free — `send_wordpress_google_drive_request` (`client.rs:1075-1109`) issues a single
`.post()` (`client.rs:1083`) — but it is a WordPress-embedded Google Drive folder *listing* call
used for crawl discovery, not a records-request submission, and no outbound email code exists in
`src-tauri` (the email hits are parsers for received `.msg`/`.eml` evidence), so the filing
transport is still new capability, not reuse. And the signing key is a locally generated Ed25519
key with no identity binding (`identity_assurance = SELF_MANAGED_LOCAL_KEY_NOT_EXTERNALLY_ATTESTED`,
`src-tauri/src/integrity.rs:247`), so today's signature proves package integrity, not who filed.

Built (2026-10-01). `src-tauri/src/analysis/request_compiler.rs` compiles a casework request
into a CPRA draft scoped to its recorded record types, fields, period and identifying citations,
and derives a `RequestFilingPlan` (determination date, elapsed days, follow-ups) from the
statutory engine once a submission date is recorded; both are recomputed on every case save.
`export_request_filing_record` signs that draft, its digest, and the plan as
`onrecord-request-filing-record-v1`, and refuses a request with no recorded submission date.
Transport remains deliberately absent, and the statute set is still the single CPRA
determination clock.

## 2. Transcription: timestamped spans with absence semantics

whisper.cpp locally, diarized, with the transcript as a derived projection that never touches
the evidence. The span envelope specified in the workbench doc anticipates this: the locator
selector gains a media time range, so a quote cites recording.mp4 @ 00:14:32-00:14:41 and
survives re-transcription with a better model via the Intact/Drifted/Stale machinery that doc
defines. The superiority is the absence rule: Whisper emits per-segment confidence, and an
inaudible segment must render as inaudible, not as silence or as its best guess promoted to
fact. No competitor's transcript distinguishes "nothing was said" from "could not hear." For
body-cam and council-meeting audio, which is exactly the record class in scope, that
distinction is the evidentiary point.

Ground truth. The span envelope and the Intact/Drifted/Stale resolution are spec, not code:
`docs/investigative-workbench-architecture.md:8-36` defines the three-selector envelope, line
26 lists the locator forms a media time range slots into, and the doc's own line 4 says it was
written before any of it exists. No spans table or selector struct is in code. The nearest
shipped machinery is document-granularity, not per-span: `verify_document_provenance` re-hashes
the stored text projection and compares it against the digest recorded at extraction
(`src-tauri/src/storage/sqlite.rs:5963-6052`). The absence rule itself is already
load-bearing in the shipped schema (`documents.shannon_entropy` nullable with no default). The
dependency is the span primitive, build step 1 at
`docs/investigative-workbench-architecture.md:191`.

## 3. Handwriting: an extraction class, not more text

Apple Vision handles handwriting locally on macOS (Windows needs a different path, and Windows
has no release path yet anyway, `docs/ROADMAP.md:695`). The superior form: handwriting, print,
and stamps are distinct extraction classes with per-region confidence and image-anchored
locators. Marginalia on a records response means someone at the agency wrote on this document,
which is itself a finding. Pinpoint flattens handwriting into searchable text; OnRecord should
surface it as attributed annotation, with low-confidence reads rendered uncertain rather than
silently merged into search.

Ground truth. The Vision-on-macOS plan is documented
(`docs/investigative-workbench-architecture.md:493-500`); the shipped engine is a Tesseract
subprocess. Extraction classes exist only at document granularity: `ExtractedDocument` has four
variants (`src-tauri/src/ingestion/queue.rs:700-717`) and `ExtractionRoute` enumerates the
routes including `ImageOcr` (`src-tauri/src/parsers/format.rs:77-89`). Below that, extraction
is one flat string per document; the engine call discards geometry and confidence, and the
extraction and storage pipeline has no region entity. (The forensic auditor's `EvidenceLocator`
does carry an optional page and bounds across IPC,
`src-tauri/src/parsers/forensic_evidence.rs:36-44`, `src/types/investigation.ts:360-366`, but
its bounds are never populated and its findings are not persisted.) The missing dependency is a
region-level extraction record; per-region confidence and image-anchored locators have no
substrate until it exists.

## 4. Cross-referencing: versioned, reproducible joins

Adopt FollowTheMoney as the schema for the planned entity layer, which buys interoperability
with the whole OpenSanctions/OCCRP data ecosystem on day one. OpenSanctions publishes bulk FtM
datasets; DuckDB does the vectorized join locally. The superior property: Aleph Pro's
cross-reference is a transient hosted lookup, but OnRecord's match record should pin the
dataset version, date, and hash, with match confidence stated in basis points, the pattern
already enforced on `statutory_violations.confidence_basis_points`. A match becomes
reproducible evidence: anyone can re-run the join against the pinned dataset and get the same
answer. Nobody versions the watchlist into the finding.

Ground truth. FtM is adopted in prose only: `docs/investigative-workbench-architecture.md:271-303`
commits to vendoring `defaultModel.json` and codegen against a pinned version, but zero FtM
code exists (`followthemoney` has zero hits anywhere in `src-tauri/src` or `src`). Meanwhile a
live non-FtM entity layer already ships on the projection side, and it has grown past "fixed":
`CorpusEntityKind` (`src-tauri/src/types/corpus.rs:5-23`) now has twelve variants, including
`Person`, `Role`, `Position`, and `DocumentClass` added for policy-vs-practice comparison, and
`corpus_entity_mentions` carries character offsets and confidence
(`src-tauri/src/storage/corpus.rs:2685-2753`), so the FtM layer is a migration target, not a
green field. The basis-points pattern is real and CHECK-enforced 0-10000; the
Rust function is `cpra_calendar_confidence_basis_points`, the column it feeds is
`statutory_violations.confidence_basis_points`. One hard correction: the linked DuckDB cannot
read parquet today. `src-tauri/Cargo.toml:36` enables only `features = ["bundled"]`, and the bundled
build compiles the parquet extension only under the `parquet` feature. Enabling it is a Cargo
feature change plus a `CORPUS_SCHEMA_VERSION` bump for any new corpus-side tables.

Registry terms (checked 2026-10-01, official pages only). CSLB publishes a free "Master List of
Contractors" (License Master, Personnel, Workers' Compensation) as CSV or `.xls` from its data
portal, licence number, business name, status, dates and classifications included, with no size or
refresh cadence printed; the paid full and update files are fixed-block text at $235 per file, with
the License Master at 700,000+ records, produced in January and July with monthly updates. Neither
page carries a use restriction; the site's conditions of use point at ca.gov/use, which places state
information "in the public domain" unless marked otherwise. The Secretary of State's bizfile bulk
product exists ("BE & UCC Bulk Orders", master unloads $100, weekly unloads free, per a help guide
not reachable in full), keyed by entity number rather than any identifier this app extracts, with no
licence text anywhere on the site; the free bizfile search has a draft keyed JSON API. Cal-Access
raw data is a daily tab-delimited `dbwebexport.zip` of the database tables (over 1 GB by a
third-party report, unverified) with no terms on the page. No dataset forbids redistribution, and
only CSLB inherits an explicit public-domain statement; the importer below takes an
operator-downloaded file rather than redistributing any of them, and only CSLB is anchored because
only the licence number is an identifier the extractor reads.

Built (2026-10-01), registry anchoring. `src-tauri/src/storage/corpus_registry.rs` imports the CSLB
License Master CSV into `corpus_registry_records` beside the corpus index (schema version 11),
keyed by licence number with leading zeros removed on both sides, and records the file's SHA-256,
import instant, record count and skipped-row count in `corpus_registry_snapshots`. A lookup gives
one of four answers that the UI keeps apart: no registry for the kind, no snapshot imported, not
listed in the named file, or registered to a name and status in it. The entity graph and the
resolution review show the answer beside a licence number as "number N is registered to NAME in
file F as of D"; it anchors the number, never a party in the documents, and absence from the
License Master is stated as absence from that file, since the list omits cancelled and revoked
licences.

## 5. AI: hypotheses gated by the quote verifier

The existing rule "a statistical claim is not a legal one" generalizes to "an LLM claim is not
evidence, it is a lead." Architecture: any AI output must resolve to spans, and the UI renders
each citation through quote verification, so a hallucinated citation shows up as Stale
immediately. That is a structural anti-hallucination gate; Everlaw shows citations but never
re-verifies the quote against an immutable extraction, because it has none. Default to a local
model. Where the operator opts into a cloud model, record every byte of evidence that leaves
the machine in a disclosure ledger: which spans, when, to which endpoint. Litigators care about
that for privilege reasons, and no competitor can offer it because their evidence already lives
on someone else's server.

Ground truth. The gate does not exist yet at the granularity the item needs. Today's verifier
attests the whole projection (`verify_document_provenance`,
`src-tauri/src/storage/sqlite.rs:5963-6052`): an invented quote over an intact
projection passes untouched, which is exactly the hallucination case the gate is for. Per-span
verification is the workbench doc's unbuilt resolver. Second correction: the extraction is
tamper-evident, not immutable. Raw asset bytes are content-addressed and never overwritten
(`commit_staged_object`, `src-tauri/src/ingestion/queue.rs:5593-5633`), but the quotable text
projection `documents_fts.extracted_text` is updated in place
(`synchronize_fts_row`, `src-tauri/src/storage/sqlite.rs:7569-7581`). No AI integration or disclosure ledger exists
anywhere in the code. Dependencies, in order: the span store and resolver, then span-resolving
AI output, then the ledger.

## 6. Redaction: a span set, self-attacked before release

Immutability makes this cleaner, not harder. A redaction is a span set applied at render time;
the original is never touched, and the production's manifest records the source hash, the
redaction span list, and a stated reason per redaction, the same discipline CPRA demands of
agencies. Then the loop nobody else has: run the unredactor, the layout side-channel analysis,
and the pixelation checks against your own production before it ships. OnRecord already owns
the attack tooling; pointing it at its own output makes it the only tool whose redactions
survive its own forensics by construction.

Ground truth. The attack side is real: `src-tauri/src/parsers/unredactor.rs` exists and
recovers hidden content, and the layout side-channel and pixelation modules exist and are wired
into the forensic pipeline (`src-tauri/src/parsers/forensic_evidence.rs:172`, `188`), though those two are deterministic measurement collectors
rather than recovery tools. The producer side does not exist at all: there is no render-time
redaction, no production output path in `export.rs`, and no production manifest;
`RedactionSpan` (`src-tauri/src/analysis/cluster.rs:56-60`) is a detection heuristic, not a
producer type. OnRecord today is a read-only forensic auditor of other people's redactions. The
missing dependency is the entire production path, sitting on the span primitive.

## 7. Monitoring: change as evidence, not alert

A watch is a scheduled re-crawl plus a diff. The superior form is evidentiary: when a document
quietly disappears from a portal or its bytes change, record both fetches with instants and
hashes, producing a provable "this record was altered/removed on date D" finding.
DocumentCloud's scraper alerts on new documents; nobody records disappearance, and agencies
removing documents is a real, litigable phenomenon.

Ground truth (2026-10-02). Every fetch is an append-only row in `document_acquisitions` (URL,
redirect chain, status, headers, SHA-256, size, the digest the document held, local receipt time).
Check source re-fetches up to 100 collected documents of an active source per run, least recently
checked first, through the governed client, and records each outcome in `document_source_checks`:
same bytes, different bytes, gone (HTTP 404/410 only), or unreachable (says nothing about the
record). The collected row is never repointed, and the re-fetched bytes are hashed and discarded.
A watch's Source copies dimension reports a document whose latest informative check changed
outcome since the baseline. Still missing: a schedule (checks are operator-started), and keeping
the re-fetched bytes, which needs a referencing row so purge can release them.

## 8. Standing search: reproducible against a corpus version

Batch queries are DuckDB's home turf. A batch run records the evaluation instant and corpus
state, so "query Q against the corpus as of C returned R" is reproducible, and a standing query
re-run after each ingestion diff becomes the alerting layer, unified with the monitoring above.

Ground truth. The query side is further along than planned: the qualifier AST exists in
TypeScript today (`src/query/types.ts:30` and the parse/serialize/compile modules;
`docs/query-language.md:25-26` names it canonical), with native execution deliberately deferred
to the search rebuild (`docs/ROADMAP.md:642-656`). The corpus side is the gap: no corpus-wide
state identifier exists. `CORPUS_SCHEMA_VERSION` pins the layout, not the data, and every other
version-shaped artifact is a per-document fact. The dependency is a corpus-state identifier,
most naturally a digest over the ordered (portal_key, document_id, extracted_text_sha256) set
or an append-only ingestion diff log; the raw material (mandatory per-document digests, indexed
instants) already exists.

Built (2026-10-01), standing search. The corpus-state identifier already shipped with the research
snapshot (`get_corpus_state_digest`: sha256 over the ordered portal, document id and
extracted-text digest set) and is reused rather than duplicated. `src-tauri/src/storage/standing_queries.rs`
stores a query's `QueryAst` from `src/query` verbatim beside the flat filters it compiled to,
refuses an AST with a negated, repeated or unsupported filter so a saved query never re-runs looser
than written, and runs each query only against a corpus state its last run did not see, inside the
one connection closure that also reads the digest. A run records that digest, a digest over its
ordered match set, an exact match count, and whether the match set differs from the previous run.
Breakdowns (status, department, source, year) and clusters of byte-identical extracted text across
requests come before any row; above 10,000 matches they are scaled from a seeded fixed-size sample
and carry `ESTIMATED_FROM_SAMPLE` with the sample size, which the panel renders as ≈. A query that
reads document text is deferred while the search index is incomplete. Labelled series numbers
(resolution, ordinance, contract, permit and similar) are extracted as `SERIES_NUMBER` entities,
and `analysis/series_gaps.rs` lists short runs of numbers absent between two indexed members of a
series with three or more; the panel states each as absence from this corpus, a lead for a request
and not a finding about the agency.

## 9. Media forensics: answer the requester's question, not Amped's

Don't chase PRNU camera attribution; a requester rarely holds the camera. The requester's
question is "does this record's physical form contradict the agency's claimed timeline": a PDF
whose producer string postdates its claimed creation, double-JPEG inside a "scan," metadata
inconsistent with the response date. Formalize it: every document gets a machine-readable
authenticity assessment whose basis is stated on the confidence ladder, oriented to timeline
contradiction. Amped answers "which camera"; OnRecord answers "was this record made when they
say it was," which is the question that wins CPRA disputes.

Ground truth. The building blocks are real working code: entropy measurement
(`shannon_entropy`, `entropy_sample_bytes`, `suspicious_data_density`), JPEG/PNG segment
inventory, PDF text-layer and auxiliary-object parsing, ZIP compression auditing, and the
ordered provenance confidence ladder in `src-tauri/src/provenance.rs`. Two corrections. `predatesClassification`
is not a dating classifier: it is a vintage flag on `ProvenanceVerificationRecord` meaning the
log row was written before the classification columns existed; no temporal document-dating
logic exists anywhere in the repo. And the raster parsers preserve metadata segments without
decoding DCT coefficients, so double-JPEG detection does not exist today. The dependency is the
inference layer itself: nothing yet compares a producer string against a claimed date, computes
recompression evidence, or emits the per-document assessment; the ladder currently scores
custody and integrity, not authorship-time claims.

## The deliberate trades, kept but weaponized

## 10. Collaboration: exchange, not shared state

Because raw evidence is immutable and content-addressed, and findings already export as signed
packages, collaboration can be git-shaped: investigators exchange signed packages over any
channel, annotations are append-only span sets that merge trivially, and divergent extractions
are handled by Drifted/Stale semantics. No server ever holds the evidence, which is a stronger
claim than any cloud platform can make to a source. MAXQDA's intercoder agreement falls out for
free: two coders' span sets over identical immutable extractions compare exactly, because the
anchors are deterministic.

Ground truth. The package format is real and its version string is
`onrecord-evidence-package-v3`, which embeds the exact signed byte sequence alongside the
human-readable manifest so an independent verifier never has to trust that its own JSON encoder
reproduces the same bytes another language's encoder did; raw storage is content-addressed and
never overwritten. Three
corrections bound the rest. The package ships a report plus digests, not evidence bytes, so an
actual exchange needs the object-store payloads added to it. The layer annotations would anchor
into is mutable: `documents_fts.extracted_text` is updated in place and deleted on removal, and
the `documents` row itself is rewritten on re-extraction
(`finalize_document_extraction`, `src-tauri/src/storage/sqlite.rs:4205`, UPDATE at
`sqlite.rs:4348-4365`). And the span subsystem, including deterministic
anchors and
Drifted/Stale, is the workbench doc's explicit pre-implementation spec with zero tables or code
today. Beyond the spans dependency, package exchange needs a key and identity story; the
current signing key is explicitly unattested (see item 1).

Built (2026-10-01), operation log. `src-tauri/src/storage/casework_operations.rs` records every
saved case revision as hash-chained add/change/remove/set operations in `casework_operations`,
and `CaseworkPacket` carries the log. On a conflicting import the preview partitions the incoming
log: an add or change of an item that no local operation since the last exchange touched
commutes and `apply_commuting_case_operations` applies those as one revision; removals, field
sets and contended items stay with the item picker. The log binds to the packet's Ed25519
signature, so an operation's author is still a supplied label, and the UI says so; attested
identity remains the open dependency.

## 11. Publishing: self-verifying bundles

Emit a static, self-contained viewer around the signed package: documents, manifests, and a
browser-side verifier that re-checks hashes and C2PA on the reader's machine. DocumentCloud
embeds are trust-me; this is verify-yourself, hostable anywhere static files go. Combined with
render-time redaction, it enables selective disclosure with proof: publish the excerpt, prove
by hash tree it comes from the withheld original.

Ground truth. The two hard verification pieces exist: real C2PA validation in
`src-tauri/src/provenance.rs` (local bytes only by design, `provenance.rs:4-10`; the Trusted rung is unreachable without a
configurable trust-anchor list, `provenance.rs:37-42`, `148`, so validation tops out at Valid),
and the Ed25519-signed package whose signature, manifest sha256, and embedded report a static
verifier could re-check today. But the bundle is not self-contained: source documents are
referenced by whole-file digest, never packaged, no viewer of any kind is emitted, and the key
is unattested, so the proof chain ends at internal consistency. The excerpt-proof needs a
structure that does not exist: every digest in the system is a flat whole-object SHA-256
(`sha256_hex`/`sha256_file`/`sha256_file_async`, `src-tauri/src/integrity.rs:150-174`), which can prove only the entire withheld original.
Excerpt-plus-membership proofs require per-chunk or per-page commitments computed at ingestion.

Built (2026-10-01), browser verifier. `scripts/verify-evidence-package.html` is written beside
every signed export with the Node verifier; it is self-contained under a `default-src 'none'`
CSP and re-checks the manifest digest, the Ed25519 signature against the embedded key, and the
readable manifest against the signed bytes, for every package schema this application signs.
Committed fixtures produced by the Rust signer pin both sides together under test. It states
internal consistency only; the key is still unattested and the bundle still carries digests, not
source bytes.

## 12. API: the open file is the API

Publish a documented, stable read-only projection and the DuckDB file becomes queryable from
Python, R, or anything else natively, no REST, no rate limits, no auth dance. The one-writer
invariant stays protected because scripts touch only the projection.

Ground truth. Both load-bearing premises are real code: the corpus index is a genuinely
separate, versioned, delete-and-rebuild DuckDB projection (rebuild is the startup
reconciliation loop; `replace_outdated_corpus_index` deletes on a schema-version mismatch), and
the core has a serialized single-writer discipline: all core writes go through one persistent
connection behind a mutex (`src-tauri/src/storage/sqlite.rs:770-774`), DuckDB's own exclusive
file lock covers cross-process access, and the corpus projection has its own dedicated writer
thread (`CorpusIndex::open`, `src-tauri/src/storage/corpus.rs:325-348`). Two obstacles the thesis
missed. The projection schema is deliberately unstable: `CORPUS_SCHEMA_VERSION = 9`
(`src-tauri/src/storage/corpus.rs:52`), with revisions 3 through 9 recorded in the constant's
own comment (`corpus.rs:22-51`) and the file discarded on every bump, and no doc describes the
schema for external consumers.
And the running app holds a read-write connection on the corpus file itself
(`open_writer_connection`, `corpus.rs:936-986`), so DuckDB's file lock means a Python or R
process cannot open it during a session. The likely resolution is an exported snapshot copy (or
a periodically refreshed read-only copy) with a documented, stability-guaranteed subset of the
schema, rather than the live file.

## Sequencing

The span primitive is the load-bearing dependency: redaction production (6), transcription
anchoring (2), AI citation gating (5), and intercoder comparison (10) all sit on it. The
workbench doc confirms the ordering: spans are build step 1
(`docs/investigative-workbench-architecture.md:191`), preceded only by the step-0 purge path,
and none of the later steps can migrate away from it. Built (2026-10-03) for finding citations:
the core-DB `spans` table, the Intact/Drifted/Stale resolver (`src-tauri/src/spans.rs`), and its
rendering in "Why we believe this"; decisions and remaining gaps are in `docs/ROADMAP.md` under
"Citation spans". Note that this list extends the span's
client roster beyond what that doc commits to (its named dependents are coding, entity
mentions, block linking, threading, and canvas); the extension is deliberate and this document
is where it lives.

Near-term on existing machinery, with the audit's correction that each needs exactly one new
persistent structure rather than none: monitoring diffs (7) need the append-only per-fetch log;
standing search (8) needs the corpus-state identifier; the forensic timeline assessment (9)
needs the inference layer over the existing parsers; the read-only projection (12) needs the
snapshot-plus-stability contract, not just documentation. Cross-referencing additionally needs
the `parquet` Cargo feature before any FtM dataset can be joined (4).

Medium: transcription, FtM cross-reference, redaction production, all gated behind spans.

Far and hardest: the filing transport (1), because no outbound channel of any kind exists yet,
portal submission surfaces churn under both dominant vendors, and email filing needs careful
account integration; and package-exchange collaboration (10), which needs a key and identity
story before anything else, the current signature proving integrity but not identity.

Built (2026-10-01), timeline ledger. `src/utils/caseTimeline.ts` places case records on the
evidence timeline: filings, recorded and modeled due dates, follow-ups, productions, decisions
and commitments, each carrying its citations and their verification rung, with a per-request
statutory clock lane from `RequestFilingPlan` and a lane per question hypothesis. Two recorded
sources giving one subject different dates (a recorded response due date against the modeled
determination date; two agenda entries for one item) are listed as contradictions with both
readings. No date is extracted from a document: the span primitive (step 1 above) does not
exist, so every case date is an operator entry or a modeled derivation of one, and the view says so.
