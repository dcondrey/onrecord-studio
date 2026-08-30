# Local Analysis Architecture

Why the app is analytically empty on a local-file corpus, what will fill it, and the order that
does. Written because the diagnosis "we need an LLM" was wrong: every finding worth surfacing on
this corpus is deterministic, and the machine learning that earns its place is infrastructure for
retrieval and normalization rather than the thing that uncovers anything.

Every number below was measured on the live workspace at
`~/Library/Application Support/com.onrecord.studio/` on 2026-08-25. Each has its re-derivation
command attached so a later reader can check whether it still holds.

## 1. What was measured

**The text layer carries almost no ISO dates.** `pdftotext` over all 63 PDFs in the object store
(4,398,168 characters):

| format | occurrences | files containing |
|---|---|---|
| `YYYY-MM-DD` | 32 | 7 of 63 |
| `M/D/YYYY` | 7,811 | |
| `Month D, YYYY` | 1,241 | |
| abbreviated `Mon. D, YYYY` | 61 | |
| other | 13 | |
| **non-ISO total** | **9,126** | 46 of 63 |

`corpus::extract_iso_dates` matches `YYYY-MM-DD` only and is the sole `DATE` producer. The
cross-document timeline has essentially no input, and no amount of downstream work changes that.

**Only 9 of 70 indexed documents carry a `DATE` entity**, and one of them is a saturated
spreadsheet (below). Sixty-one documents have no date in the index at all.

**Zero textual redaction markers exist in the entire corpus.** Not one `[REDACTED]`, "withheld
under", or block glyph across 4.4M characters. `analyze_redaction_density` and its
`HEAVY_REDACTION_DENSITY_THRESHOLD = 0.45` cannot fire on professionally redacted government
releases, because the redaction is a drawn box with the text removed, not a label.

**400 segment content hashes appear in more than one document.** Exact verbatim reuse across the
corpus, already computed and stored in `corpus_segments.content_sha256`, entirely unused.

**The document simhash LSH is loose.** Four bands of 16 bits over a 64-bit simhash yields 161
candidate pairs from 70 documents. Heavy differential redaction is precisely the case that moves a
document simhash out of every band, so the document-level LSH is the wrong candidate generator for
release comparison. Segment hashes are the right one.

Re-derive:

```
duckdb ~/Library/Application\ Support/com.onrecord.studio/onrecord_studio_corpus.db -c "
  SELECT COUNT(DISTINCT m.document_id) FROM corpus_entity_mentions m
  JOIN corpus_entities e USING(entity_id) WHERE e.entity_kind='DATE';
  SELECT COUNT(*) FROM (SELECT content_sha256 FROM corpus_segments
    GROUP BY 1 HAVING COUNT(DISTINCT document_id) > 1);"
```

Copy the file before opening it with the CLI; the app holds a writer on the live one.

## 2. The entity budget defect

`MAXIMUM_ENTITIES_PER_DOCUMENT = 20_000` is a single pool, and `extract_iso_dates` runs first in
`analyze_document`. Document `6661148630021752` holds exactly 20,000 entities and **every one of
them is a `DATE`**. Zero organizations, zero statutes, zero acronyms, zero email domains, zero URL
hosts. A spreadsheet of ISO dates consumed the whole budget and every other entity in that document
was dropped without a record that dropping occurred.

That is this project's absence-versus-measurement rule violated inside the entity pipeline: a
reader cannot distinguish "this document contains no organizations" from "we stopped looking."

The fix is a per-kind budget plus a persisted truncation flag per document and kind. A truncated
extraction must be legible as truncated.

## 3. Organization extraction

Top `ORGANIZATION` values by mention count show the failure directly:

| value | mentions |
|---|---|
| `City of San Diego` | 198 |
| `The City` | 170 |
| `Agency Agreement  Revised: March` | 140 |
| `RFP – Goods, Services, & Consultants Revised: November` | 85 |
| `City,  \nContractor` | 79 |

Two distinct problems.

**Month names consumed as title tokens — closed 2026-08-30.** A calendar word may no longer end an
organization run, so `Agency Agreement Revised: March` stops at `Revised`. Written as a terminator
rather than as an exclusion from `is_title_token`: excluding calendar words outright also closes
this case, and quietly starts `May Department Stores` at `Department` and `Friday Harbor Port
District` at `Harbor`. A name missing its first word is wrong, not short. Both directions are
proven by removal — dropping the terminator brings the month back, and swapping it for the blanket
exclusion yields `department stores`.

**`The City` at 170 mentions against `City of San Diego` at 198 — still open.** This is an
entity-resolution case, not an extraction case. The schema can now express the answer:
`corpus_entities.resolution_basis` (corpus schema 7) records why mentions were merged, and every
row written today is `EXACT_NORMALIZED_MATCH`. Nothing looser is applied, and per
`investigative-workbench-architecture.md` section 4 nothing looser may be applied silently: a
heuristic merge is proposed, recorded as `HEURISTIC_SIMILARITY`, and splittable.

## 4. The redaction detector is not ready to run at ingest

`src-tauri/src/parsers/unredactor.rs` implements PDF redaction geometry against the document
structure, versioned `pdf-redaction-geometry-v2`. It is careful work: fill alpha, fill luminance,
a colour-space model, a `MINIMUM_TEXT_OVERLAP_RATIO`, bounded PII masking, and an
`unclassified_filled_rectangles` counter that refuses to silently discard what it could not
classify. It emits four signals:

| signal | fires on this corpus |
|---|---|
| `redaction_annotation_intersections` | **never**, see below |
| `PAGE_N_UNAPPLIED_REDACTION_ANNOTATIONS` | **never**, see below |
| `dark_overlay_intersections` | constantly, and wrongly |
| `invisible_text_intersections` | on every OCR'd scan |

**There are zero `/Redact` annotations in the corpus.** Measured across 58 parseable PDFs and 2,691
pages. Both annotation-based signals are dead on real released records, because a professionally
produced release has already applied and flattened its redactions.

**The dark-overlay signal cannot currently distinguish a redaction from a shaded table cell.** A
structural probe found 708 dark filled rectangles across 16 files, 671 of them with text underneath.
Reading that text shows what it actually is:

```
fill=0.19  |Chief Regional Officer Jonathan Castillo, LCSW|      <- org-chart box
fill=0.24  |PERFORMANCE OUTCOMES|                                 <- shaded table header
fill=0.17  |Piedad.Garcia@sdcounty.ca.gov|  |858-514-3191|        <- shaded contact table
```

Every one is visible content in a shaded cell. The false-positive rate is effectively total, and
the overlap ratio does not help: a table cell fully contains its own text, so its overlap score is
maximal.

The missing discriminator is **text fill luminance**. The module tracks a rectangle's fill colour
and a text run's *rendering mode*, but never a text run's *colour*. Without it, white-text-on-dark
cannot be separated from dark-text-under-dark, and those are the two cases that matter.

**`invisible_text_intersections` will fire on every scanned document.** OCR text layers are
rendering mode 3 by design. Any scanned page carrying both an OCR layer and a dark filled region
produces `PAGE_N_INVISIBLE_TEXT_BEHIND_DARK_OVERLAY`, which is the most alarming-sounding signal in
the module and, on scans, the least trustworthy.

So this must not be wired at ingest as it stands. In an application whose premise is that a finding
is a claim about evidence, several hundred false failed-redaction findings are worse than shipping
nothing. Prerequisites, in order:

1. Track nonstroking text colour alongside rendering mode; require dark-on-dark for an overlay
   intersection.
2. Exclude OCR layers from the invisible-text signal: a full-page invisible text layer correlated
   with a full-page image is a scan, not concealment.
3. Re-measure against this corpus and require a near-zero false-positive rate before it runs
   unattended.

Note what the honest headline becomes. This corpus may contain **no failed redactions at all**, and
"we examined 2,691 pages and found none, with N rectangles we could not classify" is itself a
defensible statement about evidence. `unclassified_filled_rectangles` already exists to support
exactly that claim.

`MAXIMUM_CONCURRENT_REDACTION_AUDITS = 1`. Even once corrected, calling it inline from ingest
serializes every document through a semaphore of one, so it belongs in a post-ingest background
pass.

Re-derive the probe by walking each PDF's content stream for filled rectangles whose fill luminance
is at or below 0.35 and at least 12x4 units, then extracting text-run origins that fall inside them.
Reading the extracted strings is the whole test; a count alone is what misled this analysis in the
first place.

## 5. Derivations live in the core database, not the corpus index

Analysis output is cached in one table keyed by content and by what produced it.

```sql
CREATE TABLE corpus_derivations (
  derivation_id    TEXT PRIMARY KEY,  -- stable_id(content_sha256|content_kind|derivation_kind|producer_id)
  content_sha256   TEXT NOT NULL,
  content_kind     TEXT NOT NULL,     -- RAW_FILE | EXTRACTED_TEXT
  derivation_kind  TEXT NOT NULL,     -- REDACTION_GEOMETRY | DATE_SPANS | ENTITY_SPANS | EMBEDDING | LAYOUT
  producer_id      TEXT NOT NULL,     -- 'pdf-redaction-geometry-v2' | 'gliner-large-v2.1@int8'
  producer_class   TEXT NOT NULL,     -- DETERMINISTIC | PINNED_MODEL | UNATTESTED
  executed_on      TEXT NOT NULL,     -- LOCAL | MODAL
  payload          JSON NOT NULL,
  payload_sha256   TEXT NOT NULL,
  computed_at      TIMESTAMP NOT NULL
);
```

**It must live in `onrecord_studio_core.db`.** `replace_outdated_corpus_index` deletes the corpus
file whenever `corpus_schema_version` mismatches `CORPUS_SCHEMA_VERSION`, which is correct for a
derived index and fatal for a cache. Rows here can represent GPU-hours or billed remote compute;
they have to outlive every corpus rebuild. Keying by `content_sha256` makes co-location unnecessary.

`content_kind` is explicit rather than implied by `derivation_kind` because redaction geometry
derives from the raw file while entity spans derive from the extracted text. Leaving that mapping
implicit makes an exported row unverifiable without a private lookup table.

`payload_sha256` is the entire mechanism by which two machines can prove they agree on a derivation
without trusting each other.

`producer_class` carries evidentiary standing and nothing else. `executed_on` is metadata: a pinned
model run on rented hardware produces the same row as one run locally.

## 6. Attestability governs which models are allowed to matter

A pinned local model is a deterministic function of the bytes. Same document, same model version,
same spans, on anyone's machine, forever. An opposing party can re-derive the finding. A remote LLM
API cannot be re-derived: weights change, versions are deprecated, and nothing proves what was
returned.

So local inference is the evidentiary tier and remote LLM output is a leads tier that must be
recorded `UNATTESTED` and must never share a confidence rung with a structural finding.

This holds **only if the pinning is real**: model id, version, and quantization on every derived
row, plus a recorded digest of the ONNX file. Without that, local inference is merely a cheaper
form of the same unattestable thing.

Modal is an exception in the useful direction. A content-addressed container image plus pinned
weights is re-derivable, so Modal maps to `PINNED_MODEL` with `executed_on = MODAL`. The constraint
that follows: Modal runs the same pinned model versions as the local accelerated tier. It is
hardware the user does not own, not a different and better model set.

## 7. Capability tiers

Tiers are defined by the model set they can run, never by a hardware score. `producer_id` is the
source of truth; the tier integer is a settings-UI label. Coverage is always the query "which
documents lack a row for producer P", which never consults the ordering.

This matters because tier is not a total order. An RTX 5090 has 32GB of VRAM and no unified memory;
an M4 Max has 128GB of unified memory and no CUDA. Neither dominates.

| tier | gate | adds |
|---|---|---|
| 0 | none | deterministic only; a fully supported configuration |
| 1 | ~1GB, CPU | bge-small int8, GLiNER-small-v2.1 int8 |
| 2 | ~3GB, working GPU EP | bge-base, GLiNER-large-v2.1, reranker, layout |
| 3 | ~8GB | fp16 throughout, document VLM at int4 |
| 4 | 24GB+ VRAM or 64GB+ high-bandwidth unified | VLM at fp16, 32B-class extraction |
| 5 | Modal | tier-4 models on rented hardware, batch scale |

Three rules keep this from breaking the evidence model:

1. **Hardware selects capacity; it never silently rewrites a finding.** A tier upgrade writes a new
   row. It does not overwrite an existing one. A GLiNER-small span stays a valid GLiNER-small
   observation.
2. **The deterministic core is tier-invariant and runs everywhere.** Redaction geometry, dates,
   hashes, statutes, Bates sequences are identical on every machine. The app's core claims never
   depend on hardware.
3. **Coverage is displayed, never hidden.** The UI must be able to state "41 documents have no
   entity spans from `gliner-large-v2.1`." A user on modest hardware must not believe they have
   seen everything.

Detect by probing, not by reading specs: read memory and cores, then actually initialize the
execution provider and run a fixed benchmark. Persist the achieved tier; do not re-probe per
launch. A failed provider initialization falls back silently, because a tier misdetection must
never block ingest.

**Unverified and gating.** `ort`'s CoreML execution provider falls back to CPU per operator, and
GLiNER's DeBERTa-v3 backbone uses disentangled attention whose operators may not map. Whether tiers
2 and 3 are minutes or tens of minutes on Apple Silicon is unmeasured. One model, one document,
ops-placed-on-GPU and wall clock, before any tier claim is made in the UI.

## 8. Model licensing

GLiNER v0 and v1, including `gliner_base` and `gliner_multi`, are **CC-BY-NC-4.0** and cannot ship
in a commercial application. Only the **v2.1** line and the PII model are Apache 2.0. Pin to
`gliner_*-v2.1`. The BGE embedding models are MIT.

Models download on first use, per model, checksummed against a recorded digest. They are never
bundled in the installer; tier 2/3 is roughly 1-2.5GB in total.

## 9. Provider integration, when it happens

Consumer-subscription OAuth is closed by the providers themselves. Anthropic prohibits
subscription OAuth in third-party tools (documented February 2026, enforced April 2026). Google
banned Gemini CLI token proxying in February 2026 and suspended accounts for it. OpenAI's
"Sign in with OpenAI" is Codex-scoped and does not provision API billing.

OpenRouter PKCE is the one real login-instead-of-key path, it works from a localhost callback on
any port, the user's own balance pays, and it fronts every major model provider. One integration
satisfies both "no key pasting" and "as many providers as they want." A plain key field covers
users who already hold a direct key.

## 10. Order of work

Every finding a journalist would care about lands in units 1 through 3, and none of them need
machine learning. That is deliberate: it makes the plan robust to `ort` native linking failing
across the CI matrix, which is the largest single risk in it.

1. `corpus_derivations` in the core database, with natural-language date extraction as its first
   producer. Per-kind entity budgets with a recorded truncation flag. This is the largest
   measured gap and the cheapest to close.
2. Month names excluded from organization title-token runs, then entity resolution over the
   surviving values. `The City` and `City of San Diego` are one actor.
3. Differential release comparison over shared segment hashes, with token-shingle alignment for
   text that has shifted. Gap detection from Bates sequences and page-count discontinuities.
4. Text fill luminance and the OCR-layer exclusion in `unredactor.rs`, measured against the corpus,
   then a post-ingest background pass once the false-positive rate is near zero. Ordered here and
   not first because the measurement in section 4 showed the detector is not ready, which is the
   single largest correction this analysis produced.
5. The CoreML spike. Everything after this is gated on its result.
6. Embeddings, GLiNER v2.1, layout model, VLM tier, Modal.

## 11. The fork underneath all of this

Units 1 through 3 are document-level and touch none of the request-lifecycle machinery. The
Overview shows four zeros on a local-file workspace because every metric keys off agency,
`closed_at`, and deadline, which a file import does not have. There are two products in this
codebase and a local-file corpus exercises only one of them. The plan above is correct under either
resolution, but the question is real and it is what open item 3 is actually about.
