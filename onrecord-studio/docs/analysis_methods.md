# OnRecord Studio - Forensic Analytical Methodologies & Calculation Specifications

This document defines the calculations the Rust backend performs over ingested records. Every
threshold below is a named constant in the source; the citation after each one is authoritative,
and this document is wrong if they ever disagree.

Storage note: DuckDB is the persistent store. SQLite appears only in `parsers/database.rs`, which
opens investigator-supplied SQLite files read-only, and in the one-way upgrade path in
`storage/legacy_import.rs`.

---

## 1. Chronological Timeline Anomaly Detection

The native Rust backend analyzes public record timestamp histories to detect structural signs of data manipulation or administrative backlog trimming.

### A. Chronological Discrepancy Scans
The system compares creation and modification timestamps to identify logical sequence errors:
\[\Delta t_{\text{skew}} = t_{\text{updated\_at}} - t_{\text{created\_at}}\]
\[\text{Condition Flag} = \Delta t_{\text{skew}} < 0 \implies \text{UPDATE\_BEFORE\_CREATION}\]

\[\Delta t_{\text{lifespan}} = t_{\text{closed\_at}} - t_{\text{created\_at}}\]
\[\text{Condition Flag} = \Delta t_{\text{lifespan}} < 0 \implies \text{CLOSE\_BEFORE\_CREATION}\]

### B. Arbitrary Closure Index (Fast-Drop Filtering)
Agencies may quickly dismiss incoming public interest requests to artificially lower their open-ticket metrics. The system flags this behavior by measuring response times against a strict processing window:
\[\text{FastDrop} = (\Delta t_{\text{lifespan}} \le 300\text{ seconds}) \land (\text{Status} = \text{"Closed"})\]
* **Constant:** `SUSPICIOUS_CLOSURE_THRESHOLD_SECONDS = 300` (`analysis/timeline.rs`).
* **Triage Routing:** Triggering this flag marks the record as `SUSPICIOUSLY_FAST_CLOSURE` and ranks it higher in the inspector's manual audit list.

---

## 2. Multi-Variable Departmental Outlier Detection (OLAP Core)

The analytical engine uses DuckDB to calculate citywide performance metrics, identifying specific agencies with statistically abnormal delays.

```text
[ DuckDB core (persistent, on disk) ]
              |
              |  live sibling connection (Connection::try_clone)
              v
[ AnalyticsEngine ] -- group-by aggregation: mean closure latency per agency
              |
              v
       global mean and population standard deviation
              |
              v
       per-agency Z-score -> outlier flag
```

The analytics node must read through a sibling connection of the writer, never through a separate
connection that `ATTACH`es the core file: an attaching connection observes a snapshot frozen at
attach time and would report the database as it stood when the process started.

### A. Global Variance Formulations
The system computes the global average response latency (μ) and standard deviation (σ) across all processed cases:
\[\mu_{\text{global}} = \frac{1}{N}\sum_{i=1}^{N}\Delta t_{\text{lifespan}, i}\]

\[\sigma_{\text{global}} = \sqrt{\frac{1}{N}\sum_{i=1}^{N}(\Delta t_{\text{lifespan}, i} - \mu_{\text{global}})^2}\]

### B. Departmental Z-Score Ranking
The system monitors departmental performance by calculating Z-scores relative to citywide processing averages:
\[Z_{\text{agency}} = \frac{\mu_{\text{agency}} - \mu_{\text{global}}}{\sigma_{\text{global}}}\]
* **Classification:** one threshold is implemented. An agency is reported as a statutory-deadline
  outlier when \(Z \ge 2.0\) **or** its statutory violation count is non-zero
  (`STATUTORY_DEADLINE_Z_SCORE = 2.0`, `analysis/anomaly.rs`). There is no intermediate tier.
* \(\sigma_{\text{global}}\) is the population standard deviation, and the Z-score is defined as
  zero whenever the sample size is zero or \(\sigma_{\text{global}} = 0\), so a single-agency
  corpus never reports an outlier.

---

## 3. High-Density Text Redaction Analysis

When files are parsed, the system measures the density of redacted text blocks relative to standard content to identify hidden information dumps.

\[\text{Redaction Ratio} (R_d) = \frac{\text{Number of characters covered by redaction matches}}{\text{Total character length of the extracted text}}\]

The numerator counts *characters*, not placeholders: `[REDACTED]████` scores 1.0, not 2/14.
* **Pattern kinds** (`RedactionPatternKind`, `analysis/cluster.rs`), matched by a hand-written
  scanner rather than a regular expression engine: `ExplicitRedactedLabel`,
  `WithholdingExemption`, and `BlackUnicodeBlock` (runs of `U+2588`).
* **Triage Routing:** Files where \(R_d > 0.45\) are flagged `HEAVY_REDACTION_DENSITY`
  (`HEAVY_REDACTION_DENSITY_THRESHOLD = 0.45`, `analysis/cluster.rs`), pointing investigators to
  files that may warrant legal review or a formal appeal.

---

## 4. Local NLP Keyword Clustering & Density Maps

To isolate records tied to systemic homelessness or housing rights violations, the engine processes unformatted description text through a case-insensitive keyword filter.

### A. Word-Stem Target Matrix
Matching is case-folded against `enforcement_keyword` and the bounded two-token phrase scanner in
`enforcement_keywords` (`analysis/cluster.rs`). Six labels are implemented; each maps its accepted
surface forms to one stable label:

| Label | Accepted tokens |
|---|---|
| `sweep` | `sweep`, `sweeps`, `sweeping`, `sweeped`, `swept` |
| `encampment` | `encampment`, `encampments` |
| `eviction` | `evict`, `evicts`, `evicted`, `evicting`, `eviction`, `evictions` |
| `clean` | `clean`, `cleans`, `cleaned`, `cleaning`, `cleanup`, `cleanups` |
| `displace` | `displace`, `displaces`, `displaced`, `displacing`, `displacement`, `displacements` |
| `property storage` | exact adjacent phrase `property storage` or `property stored` |

### B. Density Calculation
$$\text{Keyword Density} (D_k) = \frac{\text{Total Count of Keyword Matches}}{\text{Total Count of Words in Case Description}} \times 100$$
* **Application:** High $D_k$ scores filter out generic municipal maintenance requests, isolating target enforcement events across the dataset.

## 5. Prose Figures Checked Against Imported Tables

`analysis/claim_validation.rs`, run by the `claim_data_mismatch` detector; findings carry the kind `CLAIM_CONTRADICTS_DATA`.

### A. What is extracted
A sentence in a public document becomes a **quantitative claim** when a token-phrase cue (never a regular expression) names a measure and a figure: a stated total (`the adopted budget of $1.2 billion`), a percent change against a prior year (`spending fell 12% from the prior year`), a within-budget statement, or a balance statement (`balanced`, `surplus`, `deficit`). Each claim carries the **scope its own words state**: fiscal year, fund, department, and the basis (`proposed`, `revised`, `adopted`, or actual spending). A sentence stating no fiscal year inherits the last one named in the document's opening (its heading, the first 600 bytes); the claim records that the year was inherited, and confidence is lower for it (§D). A sentence naming two dollar amounts or two percentages (`grew from $1.0 million to $1.2 million`) is a comparison, not one figure, and yields no claim. A hedge (`about`, `at least`, `under`) is recorded with the figure.

### B. How a claim is bound to data
A claim is evaluated only against datasets whose jurisdiction matches the document's source, through the existing financial mapping roles (`budget`, `actual`) and a `FinancialScope`. Fund and department names must match exactly one distinct value in the bound dataset. Every comparison runs through `execute_restricted_sql`; the query ids and the aggregated rows are the evidence, so a reader can re-run the same query.

### C. Tolerance
A prose figure is compared at the precision it states, not to the cent:
$$\text{band} = \pm \tfrac{1}{2}\,\text{unit of the last significant digit stated}$$
so `$1.2 billion` is consistent with any total in $[\$1.15\text{B}, \$1.25\text{B}]$; `about` doubles the band, `at least`/`under` make it one-sided. A figure outside its own band is a **mismatch**; inside it is **consistent**.

### D. What a finding does and does not claim
* A `CLAIM_CONTRADICTS_DATA` finding states only: *this sentence's figure, at the precision it states, falls outside the aggregate of the rows it binds to*, with the bound scope, the band, the observed value, and the query ids. It is a pattern signal about text versus table (`is_legal_claim` is false). It is not a statement of intent, error, or law.
* A claim that cannot be bound is **unreconciled**, never a contradiction: no dataset for the jurisdiction, ambiguous fund or department, no fiscal year, no stated budget-versus-actual basis, a partial or rejected-row dataset, or a claim kind the data cannot support. Balance, surplus and deficit claims are unreconciled today because no mapped role distinguishes revenue from expenditure; an account-number convention is not inferred.
* Confidence is derived from what was observed: 6000 basis points for a mismatch whose scope was completed from context, +1500 when the sentence named the fund, +1500 when it stated the fiscal year itself rather than inheriting the heading's. A complete dataset is a precondition, not a bonus. Confidence is unassessed whenever the claim is unreconciled.
* Consistent and unreconciled outcomes are returned by the sweep alongside mismatches so a reader can see what was checked and what could not be; only mismatches are stored as findings.
