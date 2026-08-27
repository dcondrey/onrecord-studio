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
Matching is exact-token, case-folded, against the list in `enforcement_keyword`
(`analysis/cluster.rs`). Three stems are implemented; each maps its surface forms to one label:

| Label | Accepted tokens |
|---|---|
| `sweep` | `sweep`, `sweeps`, `sweeping`, `sweeped`, `swept` |
| `encampment` | `encampment`, `encampments` |
| `eviction` | `evict`, `evicts`, `evicted`, `evicting`, `eviction`, `evictions` |

FEATURE(analysis): `clean`, `displace`, and `property storage` are not implemented. Adding a
multi-word stem needs phrase matching, which the current single-token scanner does not do.

### B. Density Calculation
$$\text{Keyword Density} (D_k) = \frac{\text{Total Count of Keyword Matches}}{\text{Total Count of Words in Case Description}} \times 100$$
* **Application:** High $D_k$ scores filter out generic municipal maintenance requests, isolating target enforcement events across the dataset.
