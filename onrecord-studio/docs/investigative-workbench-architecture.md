# Investigative Workbench Architecture

The plan for qualitative coding, entity extraction, the knowledge graph, block-level linking,
document threading, the canvas, and the transform pipelines. Written before any of it exists,
because five of those seven features are the same primitive wearing different clothes, and
building them as seven subsystems would be the most expensive mistake available here.

## 1. The one primitive: a span

MAXQDA codes a passage. Obsidian links a block. LiquidText pulls an excerpt onto a canvas.
Aleph highlights an entity mention. Heptabase threads cards. Every one of those is a durable
reference to a region of a document. Build the reference once.

The constraint this app has that Obsidian and Logseq do not: **they anchor into markdown they own
and may rewrite. We anchor into immutable evidence we must never touch.** An anchor here therefore
cannot be a position alone, because the position is not ours to keep stable. Re-OCR, a better
extractor, or a re-ingest of a revised release all move it.

So a span carries three redundant selectors, the W3C Web Annotation envelope that Hypothesis has
already proven in the field:

| selector | holds | survives |
|---|---|---|
| position | `start_offset`, `end_offset` into the canonical extraction | nothing; it is the fast path |
| quote | `exact_text`, plus 48 chars of `prefix_text` and `suffix_text` | re-extraction, minor OCR drift, reflow |
| locator | PDF page + rect, sheet + cell range, email MIME part, archive member path | extractor replacement entirely |

Resolution is: trust position, verify against quote. They agree, the span is `Intact`. They
disagree but the quote is found nearby, the span is `Drifted` and reports the new offsets without
silently persisting them. The quote is gone, the span is `Stale` and renders as stale.

`Stale` is not an error and not a zero. It is the annotation form of the rule in CLAUDE.md that
absence and measurement never share a representation: a code, a link, or a canvas card whose
evidence can no longer be located must say so rather than point at whatever text now occupies
those byte offsets. A tool that silently re-points an investigator's coded passage at different
words is worse than one that loses it.

## 2. Three tables carry six features

```
spans           a region of one document, with the selector envelope above
entities        a resolved thing: a person, an organization, an email address, a corporate id
edges           a typed, basis-labeled relationship between any two of {document, span, entity, card}
```

Then the features are views:

| feature | is |
|---|---|
| QDA coding | `codes` (a hierarchy) joined to spans through `code_applications` |
| entity extraction | `entity_mentions`: span to entity, one row per observed occurrence |
| knowledge graph | `entities` as nodes, `edges` as arcs |
| block-level linking | an edge whose endpoints are two spans, asserted by the investigator |
| document threading | edges observed from `In-Reply-To` / `References` / archive containment |
| canvas | `canvas_cards`, each pointing at a span, document, or entity; layout only |
| diagram synthesis | a rendering of a filtered subgraph; it stores nothing new |

Adding the seventh feature must not add a seventh subsystem. If it cannot be expressed as spans,
entities, and edges, that is the signal to stop and reconsider, not to add a table.

## 2a. What already exists, and the boundary that governs where new tables go

Before adding anything, note that `storage/corpus.rs` (`CORPUS_SCHEMA_VERSION = 6`) already
implements a large part of this:

| already built | where |
|---|---|
| document segmentation, 2048 chars with 192 overlap | `corpus_segments` |
| term postings with frequency | `corpus_term_postings` |
| SimHash per document and per segment, plus 4-band LSH | `corpus_documents`, `corpus_lsh_bands` |
| entities with a normalized/display pair, unique per (kind, value) | `corpus_entities` |
| entity **mentions carrying character offsets and confidence** | `corpus_entity_mentions` |
| same-segment co-occurrence edges retaining their source offsets | `corpus_entity_relationships` |
| per-kind truncation rows, so "none present" and "stopped looking" differ | `corpus_entity_truncations` |
| materialized distinct-document counts per entity | `corpus_entity_document_counts` |

Six entity kinds exist: `STATUTE`, `DATE`, `EMAIL_DOMAIN`, `URL_HOST`, `ACRONYM`, `ORGANIZATION`.
Missing for leak and FOIA work: `PERSON`, a full `EMAIL_ADDRESS` rather than only its domain,
`PHONE`, `CORPORATE_ID`, `MONEY`, `CASE_NUMBER`, `ADDRESS`, `BANK_ACCOUNT`.

**The boundary.** The corpus index is a *rebuildable projection in a separate database file*, and
`replace_outdated_corpus_index` **deletes that file outright** whenever `corpus_schema_version` no
longer matches the constant. That is correct for a projection and it is the rule that decides where
every new table goes:

| kind of data | lives in | because |
|---|---|---|
| extraction, segments, postings, LSH, entity mentions, co-occurrence | corpus DB | derived from the documents and re-derivable at any time |
| spans, codes, code applications, memos, investigator links, canvas boards and cards, entity merges the investigator confirmed | **core DB** | authored by a human and not re-derivable from anything |

An investigator's coded passage, link, or memo must never live in a file the app is entitled to
delete on a version bump. Correspondingly, **a core-DB span must never reference a corpus
`entity_id`**: that identifier is a projection artifact and does not survive a rebuild. It
references the durable pair `(entity_kind, normalized_value)`, or a core-DB entity row that the
projection is matched against.

The next bump of `CORPUS_SCHEMA_VERSION` is where the additional entity kinds land. It costs a
full reindex of the corpus and nothing else, which is exactly what a projection is for.

## 3. Observed, computed, asserted: the three bases

This is an evidence application, so the graph cannot be a pile of undifferentiated arrows. Every
edge and every entity carries how it came to exist, and the reader renders the three differently.

**Observed.** A fact read out of the bytes. `In-Reply-To: <id>` matching a `Message-ID` in another
ingested message. A file's membership in a ZIP. An attachment's parent message. A literal string
that matched a validating extractor. These are as strong as the document.

**Computed.** A derivation the app performed and can re-derive. Two entities co-occurring in six
documents. A similarity cluster. A temporal correlation. Real, reproducible, and not a claim about
the world.

**Asserted.** The investigator drew it. A link between two passages, a merge of two name variants,
a manual entity type. Carries the author and the time.

The existing precedent for this is `DeterminationBasis` in `analysis/statute.rs`, which already
refuses to let a proxy masquerade as a sourced event. Follow it exactly, and never let a computed
edge and an observed edge be the same color on the graph.

## 4. Entities without a model

There is no LLM in this application. That
is a deliberate posture and this plan keeps it. Entity extraction is therefore deterministic, and
that turns out to be the stronger design for evidence rather than a compromise:

**Extractors are validating, not merely matching.** An email address is parsed. A phone number is
parsed to E.164 with a plausible NANP area code. An EIN, a routing number, a California corporate
number, a case number, an ISO date, a currency amount: each has a shape and most have a check
digit or a range constraint. Every mention records `extractor_id`, `extractor_version`, and a
validation grade (`CheckDigitPassed`, `FormatValidated`, `PatternOnly`, `GazetteerHit`). A
`PatternOnly` hit is a lead, not a finding, and the UI must not present it as one.

**Names are the hard case and get the honest treatment.** Person and organization detection
without a model comes from structural position (email `From`/`To` headers, signature blocks,
letterhead, docket captions, a signature line above a typed name) plus corporate-suffix detection
plus an investigator-maintained gazetteer, not from guessing at capitalized word sequences in
running prose. Recall will be lower than a model's. Precision will be far higher, every hit will
have a stated reason, and the gazetteer improves with the investigation instead of being frozen at
training time.

**Mention and entity are separate rows, permanently.** A mention is observed: this exact string
occurred at this span. An entity is the merge, and merging "Robert Chen", "R. Chen", and
"rchen@example.gov" is inference. `resolution_basis` records which: `ExactNormalizedMatch` for a
byte-identical normalization, `HeuristicSimilarity` for anything looser, `InvestigatorConfirmed`
once a human agrees. A heuristic merge is proposed, never applied silently, and is always
splittable.

## 5. Search has to be rebuilt, not extended

The current search is honest about what it is and it is not enough for this. Per
`storage-and-evidence-architecture.md`, `documents_fts` is a plain table scanned with a bounded
`LIKE` on a single reduced term: no tokenization, no ranking, no Boolean operators. Fuzzy search,
cross-document correlation, and entity-scoped retrieval all need a real index.

The target is a token table with positions, which the corpus index is already shaped to hold,
plus trigram similarity for the fuzzy path, plus phonetic keys for name variants. The critical
property to preserve is the one commit `ef95155` established: when the index is incomplete, the
query refuses to answer rather than under-reporting. A leak-analysis tool that quietly returns
four of nine hits is not a slower correct tool, it is a wrong one.

## 6. Transform pipelines

"Twenty PDFs compiled into a spreadsheet" and "spreadsheet rows cross-referenced against invoice
PDFs" are the same operation: extract typed records from documents, then join. The engine is a
saved, versioned, re-runnable definition, not a button:

```
source     which documents (a query, the same AST src/query already compiles)
extract    named fields, each an extractor bound to a span, so every output cell keeps its anchor
shape      one row per document, or one row per match
join       against another extraction, or an ingested spreadsheet, on named keys
output     a table view, a CSV, an XLSX
```

Two hard requirements. **Every output cell retains the span it came from**, so a number in the
spreadsheet is one click from the page and rect it was read off. And **the definition is saved
with its version and re-runnable**, so a result is reproducible and a corpus change re-runs it.
This is what separates the feature from twenty single-purpose buttons, and it is why the
extraction language shares the query AST rather than being a second grammar beside it.

Writing spreadsheets is new capability: `calamine` reads and cannot write, and no writer crate is
present. That is a dependency decision, not a design one.

## 7. Order of construction

Each step is usable on its own and none of the later steps can migrate away from step 1.

0. **Source and document management.** Remove a source, remove one ingested document, toggle a
   source's ingestion. Built as a single purge path that every dependent table registers into, so
   that adding a table below means adding one line rather than hunting call sites.
1. **Spans.** The selector envelope, the resolver, the three states, selection in the viewer.
2. **Entity extraction.** Validating extractors, mentions, entity resolution with a basis.
3. **Edges.** Email threading and archive containment first, because both are observed and cheap.
   Co-occurrence after, clearly labeled computed.
4. **Graph view.** Force-directed, in hand-rolled SVG; no chart library exists in this tree.
5. **QDA.** Codebook, coding, memos, retrieval, co-occurrence matrix, all on spans.
6. **Canvas and threading.** Layout over the same spans and edges.
7. **Search rebuild.** Tokens with positions, trigram fuzzy, phonetic name keys.
8. **Transform pipelines**, and the spreadsheet writer they require.
9. **Diagram synthesis**, as a rendering of a filtered subgraph.

## 8. Schema constraints this tree imposes

DuckDB refuses constrained `ADD COLUMN`, refuses `ADD CONSTRAINT` entirely, and refuses
`DROP COLUMN` on `portals`, `documents`, and `legal_statutes`. Every table above is therefore new
and keyed by `document_id`. **Nothing here widens `documents`.** A new table gets its real
constraints in `0001_initial_schema.sql`, and any upgrade shim gets type-and-DEFAULT columns only.

Every table above also needs a migration test that builds the *old* shape directly, per the rule
that every test opens a fresh database and so covers nothing about an upgraded one.

## 9. The model question, decided

Section 4 above specifies deterministic extraction. The proposal on the table is `ort` +
`tokenizers` running a quantized ONNX NER model for PER/ORG/LOC/DATE/MONEY. That is the correct
mechanism if a model is used at all: it is fully local, needs no LibTorch, and the weights can be
vendored into the bundle so nothing is downloaded at runtime. `rust-bert` is rejected for the
opposite reason, LibTorch is a large native dependency for the same result.

**The decision: both, with an unbreakable seam between them.**

Rule-based extraction and model extraction are not competing implementations of one feature. They
produce different *kinds* of claim and this application already has the vocabulary for that
distinction:

| | rule-based | model |
|---|---|---|
| what it finds | case numbers, tracking ids, phones, emails, EINs, routing numbers, currency, ISO dates | person and organization names in running prose |
| failure mode | misses a novel format | asserts a person who is not one |
| basis recorded | `Observed`, with extractor id, version, and validation grade | `ModelInferred`, with model id, model version, and the token span |
| may feed a statutory finding | yes | **never** |
| default state | applied | proposed, pending investigator confirmation |

A regex that matches `\$[0-9,]+\.[0-9]{2}` found a dollar amount, and that is a fact about the
bytes. A transformer that labels "Madison" as `PER` has produced a hypothesis about a street. In a
tool whose output is meant to survive a challenge, those cannot render the same way, and a
model-labelled name must not silently become a node the graph then draws relationships from. It
becomes a *candidate* mention that an investigator confirms, and the confirmation is what promotes
it, recorded as `InvestigatorConfirmed`.

This is not a hedge added for this feature. `analysis/statute.rs` already refuses to let a proxy
date masquerade as a sourced determination. **Any NER model added here must first pass a
held-out evaluation** reporting error rate, abstention rate, calibration error, per-subgroup
metrics, and a privacy leakage count, and its measured per-class precision is shown to the
investigator rather than assumed. A model whose ORG precision on this corpus is 0.71
is useful and must say so.

Corrections to the pipeline sketch, in the order they would bite:

- **Chunking must be token-aware, not character-aware.** The corpus already segments at 2048
  characters with 192 overlap, and that segmentation is load-bearing for co-occurrence. A 512
  *wordpiece* window is a different unit and does not divide evenly into it. The `tokenizers` crate
  gives the real count; do not add a second character-based splitter beside the existing one.
- **Language detection is a real requirement and it is cheap.** `whatlang` is pure Rust and small.
  Running an English NER model over Spanish correspondence produces confident nonsense, and the
  honest behavior is to abstain on a document whose detected language the model was not trained
  for, recorded as an abstention rather than as zero entities found.
- **`lazy_static` is superseded.** `std::sync::LazyLock` is in stable std as of Rust 1.80.
- **The graph does not get its own database.** Kùzu and Cozo are good at what they do, and adding
  either means a third embedded engine and a third file that can refuse to open. This tree already
  took a production outage from exactly that class of problem: a corpus index written by an older
  DuckDB reported storage version v0.10.2, silently widened `INTEGER` to `BIGINT`, and made the
  shipped app unlaunchable. DuckDB expresses this graph with recursive CTEs, and `petgraph` handles
  in-memory algorithms on an extracted subgraph. Revisit only when traversal is measured to be the
  bottleneck, on a real corpus, with numbers.
- **`redact-ner` is unverified.** I have not confirmed that crate exists; it is out for research
  and I will not plan around it until it comes back.

## 10. Prior art, adopted and rejected

### Adopt: the FollowTheMoney ontology and its statement model

FtM (MIT, © Journalism Development Network) is the vocabulary Aleph, OpenAleph and ICIJ tooling
already speak. Adopting it makes this corpus importable and exportable rather than a private format
that resembles theirs. The canonical repo has moved to `opensanctions/followthemoney`.

Vendor the generated `defaultModel.json` (the repo's own `make default-model` target produces it,
and it is what drives the TypeScript and Java bindings) and code-generate Rust types from it at
build time against a pinned version. The `ftm-types` crate exists and is MIT/Apache-2.0, but at 275
lifetime downloads it is one person's project; read it before depending on it, and prefer vendoring
the JSON either way.

Take `make_entity_id`: SHA-1 over a salt prefix plus the natural-key parts, returning nothing when
no part contributed. **Do not** take Aleph's HMAC namespacing; it isolates untrusted multi-tenant
collections, which does not exist here.

**The statement model is the important half.** FtM decomposes an entity into
`(entity_id, prop, prop_type, value, original_value, lang, dataset, origin, first_seen, last_seen)`
triples, reversibly. That is the per-value provenance ledger this application needs, and it already
keeps `original_value` beside the normalized `value`.

Its gap, and our extension: **`origin` records where a value came from, not how it was derived.**
FtM has no observed/computed/asserted axis. So the statement table gains a `derivation` column that
is `NOT NULL` **with no default**, carrying the basis plus the reference that justifies it (which
extractor and version, which model and version, which investigator and when, which saved operation).
No default, because "we do not know how this got here" is not the same fact as "the investigator
asserted it", and a column whose default is a legal value of the thing it measures cannot say
"unknown". That is the same rule that keeps `documents.shannon_entropy` nullable.

### Adopt, and go beyond: `Mention`, which stops one level short of useful

FtM's `Mention` schema is `document`, `name`, `resolved` (the LegalEntity it resolved to), plus
`contextCountry` / `contextPhone` / `contextEmail` as weak disambiguation hints. `Page` carries
`index` and `bodyText`.

**Neither carries a character offset. Aleph's finest granularity is "this name appeared in this
document."** Its UI highlighting is a re-search at display time, not stored provenance.

That is a hard limit and this application should not inherit it. "The name appears in document D"
is a materially weaker claim than "at characters 4192 to 4203 of page 7 of document D, whose
extraction hash is ...". The span envelope in section 1 is exactly this gap closed, and offsets are
only meaningful against a pinned extraction, so the extractor identity is recorded with them. FtM
already has the slots on `Document`: `processingAgent` and `processedAt`.

### Adopt: Splink's scoring model, not Aleph's

Aleph's xref scores with a heuristic weighted-property similarity and a 0.5 display cutoff. Splink
(MIT, © Ministry of Justice) implements Fellegi-Sunter properly, and the score decomposes into
per-feature evidence, which is the whole point for a tool whose findings get challenged:

```
M_obs = log2(lambda / (1 - lambda)) + SUM_i log2(m_i / u_i)
Pr(match) = 2^M_obs / (1 + 2^M_obs)
```

where `m` is the probability of the observation given a true match (data quality), `u` the
probability given a non-match (coincidence), and `lambda` the prior. Calibration: M=0 is 0.5,
M=3 is 0.9, M=7 is 0.99. Use the numerically stable branch rather than the naive form, because
`2^M` overflows f64 near M≈1024 and one exact-identifier match gets there.

The structural insight worth more than the formula: **comparison levels are discrete buckets, not
continuous scores.** A Jaro-Winkler threshold of 0.92 is a bucket boundary, and each bucket carries
its own trained `m` and `u`. Defaults: `NameComparison` at `[0.92, 0.88, 0.7]`, generic
Jaro-Winkler at `[0.9, 0.7]`, Levenshtein at `[1, 2]`.

Term-frequency adjustment is an additive correction,
`tf_weight * (log2(u_base) - log2(term_frequency))`, and it needs no training at all: it is a
`GROUP BY value` count. Rare names weigh more than common ones, which is most of the practical
benefit for name matching.

**It ports cleanly.** Splink is a SQL generator with thin Python orchestration. Blocking, comparison
vectors, term-frequency tables, weight and probability, clustering, `u` estimation by random
sampling, and even the EM expectation step are all SQL issued to DuckDB. Only the maximisation step
(normalize counts back into m and u) and the convergence loop are Python, and both are arithmetic.
DuckDB ships `jaro_winkler_similarity`, `damerau_levenshtein`, `levenshtein`, `jaccard` and
`hamming` natively.

Avoid the `splink_udfs` community extension: DuckDB community extensions are fetched at runtime from
a network repository, which an offline app cannot rely on. Register the handful of extra functions
as Rust scalar UDFs on the connection instead.

### Adopt: OpenRefine's fingerprint key, exactly

Forty lines, BSD-3, and the highest value-per-line item in the survey. The published documentation
lists the steps in a different order than the code and omits the character map; the Java is
authoritative. Order:

1. Trim, then lowercase.
2. Unicode **NFKD** normalize, then delete `[\p{InCombiningDiacriticalMarks}\p{IsLm}\p{IsSk}]+`.
3. Apply the non-diacritic map char by char: `ss` for the eszett, `ae`, `oe`, `aa`, `th`, `c` for
   the copyright sign, and the rest of the table. This is after NFKD because NFKD does not
   decompose the eszett.
4. Delete `\p{Punct}` (Unicode class) and the listed control ranges.
5. Split on `\s+`, sort, deduplicate, join with a single space.

Two consequences the documentation will not tell you: punctuation is removed **before** the split,
so a hyphenated name becomes one token rather than two, and step 3 must follow step 2.

This is the `ExactNormalizedMatch` rung of the resolution ladder in section 4, made precise.

### Adopt as a template: OpenRefine's operation recipe

OpenRefine's Extract/Apply produces a JSON array of facet-scoped operations, each naming its
`op` id, its `engineConfig` (row-based or record-based, plus the facets that scope it), its
`expression` prefixed with its language, and its `onError` policy. It is replayable against another
project. That is precisely the shape the transform pipelines in section 6 should take, and it is
"computed" provenance done properly: every derived column names the expression that produced it.
Its documented limitation is instructive too, single-cell manual edits cannot be extracted, which
is the same reason asserted facts need their own basis rather than living in a recipe.

### Reject: Aleph's architecture, and check the licence before touching its code

Aleph is Postgres plus Elasticsearch plus Redis plus a queue plus an object store, and every one of
those services exists to solve multi-tenant, multi-user, horizontally-scaled indexing. None of that
is this application. What transfers is the content-addressed archive keyed on the file hash, the
`Document -> Folder -> Pages -> Page` entity tree as the representation of an ingested file, and
`ingest-file`'s format coverage as a scoping checklist.

Two facts to act on rather than file away:

- **`alephdata/aleph` is sunsetting.** Its README states maintenance officially ends after
  December 2025. `openaleph/openaleph`, maintained by the Data and Research Center, is the live
  fork and keeps the same FtM model. Point interop at OpenAleph.
- **`alephdata/ingest-file` was archived and relicensed: MIT through 3.18.3, AGPLv3-or-later from
  3.18.4.** For an App Store binary that is a real constraint, not a footnote. Do not vendor, port,
  or bundle anything from a tag at or after 3.18.4.

## 11. Dependency roster, decided

Every entry researched against the constraint that binds this app: pure Rust, no JVM, no Python
runtime, no network at run time, embedded in a Tauri bundle beside DuckDB. Verdicts are
engineering decisions, not a survey.

### Adopting

| crate | for | note |
|---|---|---|
| `rust_xlsxwriter` | writing XLSX | pairs with the existing read-only `calamine`; by the author of Perl's `Excel::Writer::XLSX` and Python's `XlsxWriter` |
| `csv` | writing CSV | BurntSushi's, effectively standard library |
| `pdf-extract` | **per-glyph PDF coordinates** | see below, this is the important one |
| `ocrs` + `rten` | OCR | pure Rust inference, **12.3 MB of vendorable models**, gives word and line rects |
| `strsim` | edit distance, Jaro-Winkler | not abandoned, finished; a billion downloads |
| `rphonetic` | phonetic keys | Apache commons-codec port: Double Metaphone, **Daitch-Mokotoff, Beider-Morse** |
| `mail-parser` | RFC 5322 / MIME | typed `Message-ID` / `In-Reply-To` / `References`, recursive MIME, RFC 2047 decoding |
| `sevenz-rust2` | 7z | pure Rust, the maintained successor to `sevenz-rust` |
| `petgraph` | graph algorithms | components, SCC, Dijkstra, A*, Bellman-Ford, cycles, MST, max flow |
| `tantivy` | the search rebuild | the one second index that earns its keep, see below |
| `pdf-writer` + `lopdf` | redaction and Bates stamping | `printpdf` if report layout gets involved |
| `aho-corasick` | gazetteer matching | already implied by `regex`; the highest-precision entity path |
| d3-force (frontend only) | graph layout | the simulation module alone, ~30 KB, no charting framework |

**`pdf-extract` is the answer to PDF anchoring and it is not obvious from the docs.** Its
`OutputDev` trait is a callback interface, and `output_character` hands you the text rendering
matrix, the advance width, and the font size, with `begin_word`/`end_word` bracketing. That is a
per-glyph bounding box and free word grouping. The shipped plain-text, HTML and SVG outputs are
just three implementations of that trait; write a fourth that emits `(page, char, x, y, w, h)`.
`lopdf` stays for structure and for writing, but it tracks no text matrix, so using it for
coordinates means reimplementing what `pdf-extract` already did.

**`rphonetic` carries Daitch-Mokotoff and Beider-Morse**, which matter more than they look: Soundex
mangles non-Anglo surnames, and a FOIA corpus is full of them.

**`tantivy` is the only second index accepted**, and the reasoning is the opposite of the reasoning
that rejects a second database engine. It provides capability DuckDB genuinely lacks: positional
phrase queries, `FuzzyTermQuery` backed by a Levenshtein automaton intersected with the term
dictionary's FST rather than a scan, `RegexPhraseQuery`, and real BM25. Its failure mode is the
familiar one, an index written by 0.24 is unreadable by 0.26, so it gets a `TANTIVY_INDEX_VERSION`
constant and the same delete-and-rebuild-on-mismatch discipline as the corpus index. Set the
document store to `None`, since the text already lives in DuckDB. Index size overhead is not
published anywhere; measure it on a real corpus rather than quoting a number.

### Rejecting

| crate | why |
|---|---|
| `pdfium-render` | best-in-class per-character data, but wraps Google's Pdfium C++ library. Reconsider only if the pure-Rust constraint relaxes |
| tesseract bindings | libtesseract + libleptonica |
| `rapidfuzz` | no release since 2023-12-01. Adopt only if `strsim` measures as the bottleneck |
| `fuzzy-matcher` | a subsequence matcher for file pickers, not an edit-distance metric. Wrong tool for document search; fine for a command palette |
| `probabilistic_collections` | 2020, declining, and it would replace working hand-rolled SimHash and LSH with an unmaintained dependency while giving up band-count tuning |
| `fdg-sim` | stale since December 2022, abandoned workspace member |
| `rust-bert` | libtorch |
| `omniparse` | a real, license-clean Rust crate, but it wraps `lopdf`, `calamine`, `scraper` and `zip`, all of which this app already calls directly |
| `polars` | duplicated capability. DuckDB is already embedded and answers every question here |
| `kuzu` | cmake and a C++ build, a directory-based store, and a Rust binding ten months behind upstream |
| `cozo` | last Rust release 2023-12-11 |
| `surrealdb` | **BSL 1.1**, not open source, change date 2030-01-01. Plus a distributed-database dependency tree for a local app, at `3.3.0-beta` |
| `eframe` / `egui` | immediate-mode GPU canvas: no native accessibility tree, and this app has shipped accessibility claims |
| **`unrar`** | **the crate declares MIT/Apache-2.0 but that covers only the Rust wrapper.** The bundled C++ carries the UnRAR licence, which forbids using the source to build a RAR-compatible archiver. That is a field-of-use restriction, so it is not an OSI licence, and the crates.io metadata is misleading about what would actually ship |
| DuckDB `vss` extension | its own documentation: WAL recovery is unimplemented for custom indexes, so an unclean shutdown can cause "data loss or corruption of the index". Also memory-only without an experimental flag, and community extensions are fetched over the network |

**The RAR decision is yours, and it is legal rather than technical.** Default is no RAR support,
with a clear message telling the user to convert. Reverse it by accepting a non-OSI component, a
C++ dependency, and the UnRAR licence text in the bundle's acknowledgements.

### Deferred, with the numbers to decide on later

**Model-based NER.** `ort` supports true static linking through `ORT_LIB_PATH`, so a single binary
with no runtime download is achievable; its default feature downloads at *build* time, not run time,
which reads worse than it is. GLiNER small v2.1 is Apache-2.0 and publishes a full int8 / uint8 /
q4 / fp16 quantization ladder, and its zero-shot label list fits FOIA work far better than a
PERSON/ORG/LOC-locked model: you pass `["agency", "official", "statute", "case number"]` at
inference time. `rten` would keep it pure Rust but its operator coverage for a transformer graph is
unverified; it is proven only on the ocrs CNN and CRNN.

**Do the gazetteer first anyway.** For this corpus the entities that matter are largely a known
cast: agencies, named officials, statute citations, case numbers. `aho-corasick` over a curated
gazetteer plus `regex` for structured identifiers beats a general NER model on precision, costs
zero megabytes, and is auditable, which is what matters when the output is evidence.

**Semantic search is feasible offline and costs roughly 50 to 100 MB.** `fastembed` exposes
`try_new_from_user_defined`, which loads model files supplied directly rather than fetching from
Hugging Face, so vendored weights plus a statically linked ONNX Runtime gives a genuinely offline
build. For the ANN index use `hnsw_rs`, pure Rust, with the vectors stored as a plain `FLOAT[384]`
DuckDB column and the HNSW rebuilt in memory from that column when stale, so the index is derived
state with no file to corrupt. Below roughly 50k documents, skip HNSW and brute-force the cosine
similarity: 50k by 384 floats is 77 MB and a full scan is milliseconds. The reason to hesitate is
not the bundle size, it is that a semantic-similarity score is a statistical claim, and this
project's own rule says a statistical claim is not a legal one. "Similar documents" is retrieval
assistance and must be labelled as such, never as a finding.

**macOS Vision OCR.** `VNRecognizeTextRequest` is on-device, ships with the OS, adds zero bundle
size, needs no network, and is materially more accurate than `ocrs` on scanned documents including
multi-language, where `ocrs` is Latin-only and self-describes as an early preview. The cost is an
Apple framework call through objc2 rather than pure Rust. Given this is already a macOS Tauri app,
the pure-Rust constraint may have been excluding it by accident. Plan: `ocrs` as the portable
default, Vision behind a macOS feature flag, and **record which engine produced the text on the
finding**, the way Datashare records `extractor` and `extractorLanguage` on every named entity.

### PST, which is not a gap after all

I initially recorded PST and OST as the biggest format hole in the stack, on the basis that no
mature pure-Rust reader existed. **That was wrong.**

**`outlook-pst` is `microsoft/outlook-pst-rs`: a clean-room MS-PST implementation, MIT licensed,
maintained by Microsoft.** Its README states the goal plainly, a cross-platform reference
implementation compatible with the `mspst32.dll` Outlook MAPI store provider, with portability
across operating systems and architectures as a top priority. Latest 1.2.0, released 2026-03-17,
123k lifetime downloads with 79k in the last ninety days, so it is on a steep upward curve.

Its entire non-dev dependency list is **`byteorder`, `thiserror` and `tracing`**. Three pure-Rust
crates, and `thiserror` 2 is already in this tree. There is no C, no FFI, no build script pulling a
native library. This is as clean as a format reader gets, and it is a better position than the app
is in for several formats it already supports.

Its dev-dependencies are informative about the surface: `compressed-rtf` and `codepage-strings`
appear there, so the reader hands back raw message bodies and the caller decodes compressed RTF and
legacy codepages. Budget for those two crates alongside it. There is also a `ratatui` PST browser
in the examples, which is a usable reference for the folder-and-message traversal API.

**Adopt `outlook-pst`.** With `mail-parser` for `.eml` and MIME, the existing `msg_parser` for
single-message `.msg` exports, and an mbox split on `^From ` lines, that closes the email format
matrix for FOIA production.

Three name collisions on crates.io to avoid, because two of them look right and are not:

| crate | what it actually is |
|---|---|
| `outlook-pst-rw` | a real fork of the Microsoft crate by a third party adding create and incremental append, MIT, identical dependency list. Only needed if this app ever *writes* PST, which it does not. First published 2026-08-09, 625 downloads |
| `pst-rs` | **AGPL-3.0-or-later**, and version 0.0.1 describes itself as a "crates.io reservation" with 26 lifetime downloads. A placeholder, and the licence disqualifies it regardless |
| `pff`, `pst` | neither is what the name suggests. `pff` is an Nginx log parser that blocks patterns on the PF firewall; `pst` publishes posts to Micro.blog. There is no `libpff-sys` or `pff-sys` on crates.io at all |

## 12. Document model, adopted from Docling

Docling's code is Python and its models are 358 MB of safetensors with no ONNX published, so
neither is usable here. **Its schema is, and it is the best available target format.**

`DoclingDocument` is a flat content-item store (`texts`, `tables`, `pictures`, `groups`) plus a
separate tree of JSON pointers over it (`body` for main content, `furniture` for headers, footers
and page noise). Reading order is the depth-first walk of `body.children`, and it is **stored, not
inferred at read time**. Every item carries provenance:

```
ProvenanceItem { page_no, bbox, charspan }
```

**`charspan` is the load-bearing field.** Every layout block records both a page rectangle and the
character range it occupies in the serialized text, which is exactly the position-plus-locator pair
the span envelope in section 1 needs. Docling arrived at the same requirement independently, which
is the strongest evidence available that it is the right shape.

Take the table model verbatim as well: cells are a **sparse list** carrying
`start_row_offset_idx` / `end_row_offset_idx` / `start_col_offset_idx` / `end_col_offset_idx` plus
`row_span` / `col_span` and a bbox, with the dense grid derived. That handles spanned cells
correctly where a nested rows-of-cells array does not, and it maps directly onto a DuckDB
`table_cells` table. Header identity is a property of the cell (`column_header`, `row_header`,
`row_section`), not a separate block.

Borrow Unstructured's **label vocabulary** (`Title`, `NarrativeText`, `ListItem`, `Table`, `Header`,
`Footer`, `FigureCaption`, `Formula`, `PageBreak`, `PageNumber`, `Form`) but not its structure,
which is a flat `parent_id` plus `category_depth` with no character span. Its two chunking rules are
worth implementing directly and are about fifty lines: **a table never merges with surrounding
prose**, and **a title force-closes the preceding chunk even when it would have fit**.

From Tika, take one thing: **`tika-mimetypes.xml`**, Apache-2.0 and in the freedesktop
shared-mime-info format, vendorable as a data file with a Rust evaluator. And take the shape of its
`/rmeta` output, a flat array with one row per embedded document carrying a path back to its
container, which is the same design as document families below.

From Datashare, take the mention record: an **offset list** rather than one row per occurrence,
`rootDocument` alongside `documentId` so an entity found in an attachment stays attributable to the
parent email, and `extractor` plus `extractorLanguage` recorded on the finding itself.

## 13. Email threading and families

**Threading is RFC 5256 section 4, which is the standardization of Jamie Zawinski's algorithm.**
Implement the RFC, cite jwz. The container node holds a possibly-**null** message, which is the
design's whole point: a placeholder for a message whose `Message-ID` is referenced but whose copy
was never produced. In a FOIA corpus those gaps are frequently the finding.

The steps, in order: build an id table keyed by `Message-ID`, linking each message's `References`
chain oldest-first while never creating a loop and never overwriting an existing real parent; take
the root set as every container without a parent; prune empty containers, splicing out a childless
placeholder but **not** promoting children to the root when an empty container has more than one,
since that fabricates separate threads from one; then merge root containers by normalized base
subject; then sort siblings by sent date.

**Step five is the one people skip and then their threads fragment on Outlook traffic**, where
`References` is unreliable. RFC 5256 specifies base-subject normalization as seven ordered steps:
RFC 2047 decode, strip trailing whitespace and `(fwd)`, strip leading `Re:` / `Fwd:` / `Re[5]:`
prefixes and bracketed list tags, repeat until stable. `mail-parser` does the RFC 2047 decoding
that step requires.

**Inclusiveness is the highest-leverage reviewer feature in the entire survey.** An email is
inclusive when it carries unique content not wholly contained in a later message of the thread, so
reviewing only inclusive messages covers the whole conversation without hiding any text. It
collapses a forty-message chain to three documents, and it is pure logic over the thread tree.

**Families.** A family is a parent item plus everything embedded in it, transitively: an email and
its attachments, an archive and its contents, a spreadsheet embedded in a Word file. Store it as a
flat table with `parent_id`, an `embedded_resource_path`, and a `family_id` denormalized onto every
member. Three rules, in decreasing order of how badly they are usually broken:

1. **Deduplicate on a canonicalized field set, not on bytes.** Two `.msg` exports of the same email
   have different bytes and different hashes. The email dedup key is a hash over normalized sender,
   normalized recipient sets, sent time, subject, body text, and the ordered list of attachment
   hashes. Loose files dedup on file bytes. Keep both hashes; they are different claims.
2. **Dedup never breaks a family.** An attachment byte-identical to a standalone file elsewhere is
   not removed from its parent. Suppression happens at family level or not at all. Producing an
   email whose attachment list does not match what was sent is a false statement about evidence.
3. **Horizontal dedup is recorded, never discarded.** Deduplicating within one custodian is safe.
   Deduplicating across custodians destroys the fact that three people each held a copy, which is
   frequently the point. Keep the suppressed row with a pointer to the survivor and the custodian
   it came from.

For near-duplicates, keep the hand-rolled SimHash and 4-band LSH rather than adopting a stale
crate. They answer different questions and both are wanted: SimHash-64 at a Hamming distance of 3
or less is the Manku operating point for "the same document exported twice", and MinHash with
banded LSH at Jaccard 0.8 or above is "the same letter with one paragraph changed". Compute an
exact hash over normalized text separately, because that is what "duplicate" means in a production
log. Note that the vendor default thresholds circulating for this are convention rather than
documented: Relativity's published page describes a Minimum Similarity Percentage but states no
default.

## 14. Second-pass candidates, decided

### `extractous`: real, impressive, and still rejected

This one deserves a correction to section 11's flat claim that Tika cannot be used without a JVM.
**It can, and `extractous` does it.** Apache-2.0, 335k downloads in ninety days, and its README is
explicit: for formats not natively handled in Rust it compiles Apache Tika into native shared
libraries with **GraalVM ahead-of-time compilation**, then links and calls them from Rust. "No local
servers, no virtual machines, or any garbage collection." That path exists and is shipping.

It is still wrong for this application, for four reasons that compound:

1. **Its build script downloads prebuilt native libraries over the network.** `reqwest`, `flate2`,
   `tar`, `zip` and `walkdir` are all *build* dependencies. That is a reproducibility and
   supply-chain property this project should not accept, and it is invisible from the crate page.
2. **`jni` is a normal dependency**, so JNI is still in the runtime path, calling into the GraalVM
   isolate. The JVM is gone; the JNI boundary is not.
3. **More signed Mach-O binaries in the bundle.** This tree has already paid for that; commit
   `dbcb619` exists because every Mach-O in the App Store bundle has to be checked and readable.
   Adding prebuilt third-party shared libraries to that surface is a real recurring cost.
4. **Last release 2024-12-21**, roughly twenty months ago. It also pins `thiserror` 1 while this
   tree is on 2.

And the payoff is small: the formats it would add over PDF, Office, Excel, RTF, HTML, MSG, the
archive family and now PST are narrow. Revisit only if a specific format this app must read has no
Rust reader at all.

### Rejected outright

| crate | why |
|---|---|
| `exif-oxide` | **AGPL-3.0-or-later**, v0.1.0, 764 lifetime downloads. Licence alone disqualifies it for a distributed binary |
| `sn0int` | **GPL-3.0**, and it is a network OSINT framework with a script package manager. Wrong licence, wrong shape, wrong trust model for an offline evidence tool |
| `urx` | a CLI that pulls URLs from web archives. Network OSINT, no repository listed, 122 downloads in ninety days |
| `osintrs` | 540 lifetime downloads, 10 recent, no repository. Abandoned |
| `omniparse` | already rejected in section 11: license-clean and real, but a dispatch layer over `lopdf`, `calamine`, `scraper` and `zip`, all called directly here |
| `malwaredb-types` | data types for the MalwareDB project. 375 downloads in ninety days, and nothing in it addresses a question this app asks |

### Worth taking

**`kamadak-exif`** for image metadata: pure Rust, BSD-2-Clause, 2.86 million downloads in ninety
days. That is the mature default, and image EXIF is genuine evidence in a records corpus, since a
photograph's capture time and device frequently contradict the story a document tells about it.
`nom-exif` is the alternative worth knowing about because it also reads video and audio container
metadata, which `kamadak-exif` does not, but its licence field is non-standard so it needs reading
before adoption. `little_exif` is the only pure-Rust crate with EXIF *write* support, which this app
does not need and arguably must never have, since writing metadata onto evidence is the opposite of
the job.

**The `ripgrep` library crates, `grep-searcher` and `grep-regex`**, not the `ripgrep` binary crate.
Unlicense/MIT, BurntSushi, 5.2 million downloads in ninety days. They give line-oriented buffered
regex search with encoding and binary detection already handled. This is not a replacement for the
index, which answers corpus-wide questions; it is the right tool for searching *within* one
document without a round trip through the index, and for scanning the staging directory during
ingestion. Modest, cheap, and it removes a class of hand-rolled buffer handling.

## 15. Third-pass candidates

**`ulid` — adopt for every new authored table.** MIT, 12.7 million downloads in ninety days,
released 2026-07-16. A ULID is 128 bits like a UUID but lexicographically sortable by creation
time, so a DuckDB index on a span, code-application or memo id clusters by time and answers range
scans, where `uuid` v4 is deliberately random and clusters nothing. This tree already depends on
`uuid` with the `v4` feature; leave existing uses alone and use ULIDs for spans, edges, code
applications, memos and canvas cards, where "everything authored in this session" and "everything
since last week" are queries an investigator actually runs.

**`ole` — rejected, and the replacement is already here.** WTFPL, last released **2018-03-23**,
1,802 downloads in ninety days. `cfb` does the same job (Compound File Binary, the OLE2 structured
storage container) at 18.4 million downloads in ninety days, MIT, released 2026-02-13, and **it is
already in this tree** as a dev-dependency. If production code needs to read OLE2 containers,
promote `cfb` from `[dev-dependencies]` to `[dependencies]`. Do not add a second one.

**`sled` — rejected twice over.** Its latest published version is `1.0.0-alpha.124`, last released
2024-10-11: a perpetual alpha whose rewrite has not landed. Even if it were stable it is a third
embedded storage engine, which section 11 rules out on evidence this project has already paid for.

**`bufwriter` does not exist on crates.io, and should not.** `std::io::BufWriter` is in the standard
library.

**`note` does not exist. `bookmark` is a browser-bookmark manager** with 107 lifetime downloads and
no relationship to anything here. If the intent behind those two names was the notes and bookmarks
*features*, they are not dependencies: memos and saved positions are core-database tables hanging
off the span primitive in section 1, and building them on an external crate would put authored
evidence outside the durability boundary in section 2a.
